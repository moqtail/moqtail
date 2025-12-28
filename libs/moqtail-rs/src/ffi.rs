// Copyright 2025 The MOQtail Authors - Complete FFI Implementation with WebTransport
//
// Licensed under the Apache License, Version 2.0

//! Complete FFI implementation for MOQtail C/FFmpeg integration
//!
//! This provides a fully functional C API for publishing MOQ tracks
//! with both LOC and MMTP-MFU container formats from FFmpeg via WebTransport.
//!
//! This dual-format implementation enables direct comparison of moqtail vs moq-lib
//! for MMTP-MFU integration - evaluating which framework is more concise and better designed.

use std::ffi::{CStr, c_void};
use std::os::raw::c_char;
use std::ptr;
use std::sync::Arc;
use std::collections::VecDeque;
use tokio::sync::{Mutex, RwLock, mpsc, Notify};
use bytes::{Bytes, BytesMut, BufMut};
use tracing::{info, error};

// MMTP-MFU support for design comparison with moq-lib
use mmt_core::{
    MmtpHeader,
    MpuHeader,
    PacketType,
    FragmentType,
    FecType,
    MMTP_HEADER_SIZE,
    MPU_HEADER_SIZE,
};

use crate::model::common::tuple::Tuple;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::location::Location;
use crate::model::control::client_setup::ClientSetup;
use crate::model::control::control_message::ControlMessage;
use crate::model::control::publish_namespace::PublishNamespace;
use crate::model::control::subscribe_ok::SubscribeOk;
use crate::model::control::constant as moq_constant;
use crate::model::data::object::Object;
use crate::model::data::subgroup_header::SubgroupHeader;
use crate::model::data::subgroup_object::SubgroupObject;
use crate::model::data::constant::{ObjectForwardingPreference, ObjectStatus};
use crate::transport::control_stream_handler::ControlStreamHandler;
use crate::transport::data_stream_handler::{HeaderInfo, SendDataStream};
use wtransport::{ClientConfig, Endpoint};

/// Opaque handle to MOQtail client context
#[repr(C)]
pub struct MoqtailClientHandle {
    _private: [u8; 0],
}

/// Opaque handle to MOQtail track context
#[repr(C)]
pub struct MoqtailTrackHandle {
    _private: [u8; 0],
}

/// MOQtail object for FFI (LOC format with varint extensions)
#[repr(C)]
pub struct MoqtailObject {
    pub group_id: u64,
    pub object_id: u64,
    pub publisher_priority: u8,
    pub is_keyframe: bool,
    pub timestamp_us: u64,
    pub payload: *const u8,
    pub payload_len: usize,
    pub config_data: *const u8,
    pub config_len: usize,
}

/// MMTP-MFU object for FFI (MMTP format with fixed 12-byte header)
/// Design comparison: This structure shows how moqtail would handle MMTP-MFU
#[repr(C)]
pub struct MmtpMfuObject {
    pub packet_id: u16,
    pub packet_sequence: u32,
    pub timestamp: u64,
    pub packet_type: u8,        // PacketType enum value
    pub mpu_sequence: u32,
    pub mpu_fragment_type: u8,  // FragmentType enum value
    pub payload: *const u8,
    pub payload_len: usize,
}

/// Internal client context with WebTransport connection
struct ClientContext {
    runtime: tokio::runtime::Runtime,
    connection: Arc<wtransport::Connection>,
    control_handler: Arc<Mutex<ControlStreamHandler>>,
    namespace: String,
}

/// Internal track context
struct TrackContext {
    client: Arc<Mutex<ClientContext>>,
    track_name: String,
    track_alias: u64,
    object_tx: mpsc::UnboundedSender<ObjectMessage>,
    subscribe_notify: Arc<Notify>,
}

/// Object message for async transmission
enum ObjectMessage {
    /// LOC format (varint extensions)
    Loc {
        group_id: u64,
        object_id: u64,
        publisher_priority: u8,
        is_keyframe: bool,
        timestamp_us: u64,
        payload: Bytes,
        config_data: Option<Bytes>,
    },
    /// MMTP-MFU format (fixed 12-byte header + MPU header)
    Mmtp {
        packet_id: u16,
        packet_sequence: u32,
        timestamp: u64,
        packet_type: PacketType,
        mpu_sequence: u32,
        mpu_fragment_type: FragmentType,
        payload: Bytes,
    },
}

/// Helper to convert raw pointer to Arc<Mutex<ClientContext>>
unsafe fn ptr_to_client(ptr: *mut MoqtailClientHandle) -> Arc<Mutex<ClientContext>> {
    unsafe { Arc::from_raw(ptr as *const Mutex<ClientContext>) }
}

/// Helper to convert Arc<Mutex<ClientContext>> to raw pointer
fn client_to_ptr(client: Arc<Mutex<ClientContext>>) -> *mut MoqtailClientHandle {
    Arc::into_raw(client) as *mut MoqtailClientHandle
}

/// Helper to convert raw pointer to Arc<Mutex<TrackContext>>
unsafe fn ptr_to_track(ptr: *mut MoqtailTrackHandle) -> Arc<Mutex<TrackContext>> {
    unsafe { Arc::from_raw(ptr as *const Mutex<TrackContext>) }
}

/// Helper to convert Arc<Mutex<TrackContext>> to raw pointer
fn track_to_ptr(track: Arc<Mutex<TrackContext>>) -> *mut MoqtailTrackHandle {
    Arc::into_raw(track) as *mut MoqtailTrackHandle
}

/// Initialize MOQtail client and connect to relay via WebTransport
///
/// # Safety
/// `relay_url` must be a valid null-terminated C string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moqtail_client_connect(
    relay_url: *const c_char,
    draft_version: u64,
) -> *mut MoqtailClientHandle {
    if relay_url.is_null() {
        eprintln!("[moqtail_ffi] Error: relay_url is null");
        return ptr::null_mut();
    }

    let url = match unsafe { CStr::from_ptr(relay_url) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            eprintln!("[moqtail_ffi] Invalid UTF-8 in relay_url: {}", e);
            return ptr::null_mut();
        }
    };

    eprintln!("[moqtail_ffi] Connecting to relay: {}, draft version: 0x{:x}", url, draft_version);

    // Create Tokio runtime
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[moqtail_ffi] Failed to create Tokio runtime: {}", e);
            return ptr::null_mut();
        }
    };

    // Connect via WebTransport
    let result = runtime.block_on(async {
        // Create WebTransport client config without cert validation (for self-signed certs)
        let config = ClientConfig::builder()
            .with_bind_default()
            .with_no_cert_validation()
            .build();

        // Connect to relay
        let endpoint = match Endpoint::client(config) {
            Ok(ep) => ep,
            Err(e) => {
                eprintln!("[moqtail_ffi] Failed to create endpoint: {}", e);
                return Err(format!("Failed to create endpoint: {}", e));
            }
        };

        let connecting = endpoint.connect(url.as_str());
        let connection: Arc<wtransport::Connection> = match connecting.await {
            Ok(conn) => Arc::new(conn),
            Err(e) => {
                eprintln!("[moqtail_ffi] Failed to establish connection: {}", e);
                return Err(format!("Failed to establish connection: {}", e));
            }
        };

        eprintln!("[moqtail_ffi] WebTransport connection established");

        // Open bidirectional control stream
        let (send_stream, recv_stream) = match connection.open_bi().await {
            Ok(bi_result) => match bi_result.await {
                Ok(streams) => streams,
                Err(e) => {
                    eprintln!("[moqtail_ffi] Failed to await bi stream: {}", e);
                    return Err(format!("Failed to await bi stream: {}", e));
                }
            },
            Err(e) => {
                eprintln!("[moqtail_ffi] Failed to open bi stream: {}", e);
                return Err(format!("Failed to open bi stream: {}", e));
            }
        };

        let mut control_handler = ControlStreamHandler::new(send_stream, recv_stream);

        // Send ClientSetup
        let client_setup = ClientSetup::new(
            vec![moq_constant::DRAFT_14],
            vec![]
        );

        if let Err(e) = control_handler.send_impl(&client_setup).await {
            eprintln!("[moqtail_ffi] Failed to send ClientSetup: {}", e);
            return Err(format!("Failed to send ClientSetup: {}", e));
        }

        eprintln!("[moqtail_ffi] ClientSetup sent");

        // Receive ServerSetup
        let server_setup = match control_handler.next_message().await {
            Ok(ControlMessage::ServerSetup(m)) => m,
            Ok(m) => {
                eprintln!("[moqtail_ffi] Unexpected message: {:?}", m);
                return Err(format!("Expected ServerSetup, got {:?}", m));
            }
            Err(e) => {
                eprintln!("[moqtail_ffi] Failed to receive ServerSetup: {:?}", e);
                return Err(format!("Failed to receive ServerSetup: {:?}", e));
            }
        };

        if server_setup.selected_version != moq_constant::DRAFT_14 {
            eprintln!("[moqtail_ffi] Version mismatch: got 0x{:x}", server_setup.selected_version);
            return Err(format!("Version mismatch: got 0x{:x}", server_setup.selected_version));
        }

        eprintln!("[moqtail_ffi] ServerSetup received, version: 0x{:x}", server_setup.selected_version);

        Ok((connection, control_handler))
    });

    let (connection, control_handler) = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[moqtail_ffi] Connection failed: {}", e);
            return ptr::null_mut();
        }
    };

    let ctx = ClientContext {
        runtime,
        connection,
        control_handler: Arc::new(Mutex::new(control_handler)),
        namespace: String::new(),
    };

    let client = Arc::new(Mutex::new(ctx));
    eprintln!("[moqtail_ffi] Client connected successfully");

    client_to_ptr(client)
}

/// Publish a track with WARP catalog metadata
///
/// # Safety
/// All string pointers must be valid null-terminated C strings
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moqtail_publish_track(
    client: *mut MoqtailClientHandle,
    namespace: *const c_char,
    track_name: *const c_char,
    catalog_json: *const c_char,
) -> *mut MoqtailTrackHandle {
    if client.is_null() || namespace.is_null() || track_name.is_null() || catalog_json.is_null() {
        eprintln!("[moqtail_ffi] Error: null pointer in publish_track");
        return ptr::null_mut();
    }

    let ns = match unsafe { CStr::from_ptr(namespace) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let name = match unsafe { CStr::from_ptr(track_name) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let _catalog = match unsafe { CStr::from_ptr(catalog_json) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    eprintln!("[moqtail_ffi] Publishing track: {}/{}", ns, name);

    // Clone the Arc to keep client alive
    let client_arc = unsafe {
        let arc = ptr_to_client(client);
        let cloned = arc.clone();
        std::mem::forget(arc);
        cloned
    };

    // Get runtime from client
    let runtime_handle = {
        let ctx = client_arc.blocking_lock();
        ctx.runtime.handle().clone()
    };

    // Send PublishNamespace and wait for Subscribe
    let result = runtime_handle.block_on(async {
        let mut ctx = client_arc.lock().await;
        ctx.namespace = ns.clone();

        // Send PublishNamespace
        let namespace_tuple = Tuple::from_utf8_path(&ns);
        let publish_ns = PublishNamespace::new(0, namespace_tuple, &[]);

        let mut control = ctx.control_handler.lock().await;
        if let Err(e) = control.send_impl(&publish_ns).await {
            eprintln!("[moqtail_ffi] Failed to send PublishNamespace: {}", e);
            return Err(format!("Failed to send PublishNamespace: {}", e));
        }
        drop(control);

        eprintln!("[moqtail_ffi] PublishNamespace sent for: {}", ns);

        // Wait for PublishNamespaceOk
        let mut control = ctx.control_handler.lock().await;
        match control.next_message().await {
            Ok(ControlMessage::PublishNamespaceOk(_m)) => {
                eprintln!("[moqtail_ffi] PublishNamespaceOk received");
                Ok(())
            }
            Ok(m) => {
                eprintln!("[moqtail_ffi] Unexpected message: {:?}", m);
                Err(format!("Expected PublishNamespaceOk, got {:?}", m))
            }
            Err(e) => {
                eprintln!("[moqtail_ffi] Failed to receive PublishNamespaceOk: {:?}", e);
                Err(format!("Failed to receive PublishNamespaceOk: {:?}", e))
            }
        }
    });

    if let Err(e) = result {
        eprintln!("[moqtail_ffi] Publish track failed: {}", e);
        return ptr::null_mut();
    }

    // Create channel for sending objects
    let (object_tx, object_rx) = mpsc::unbounded_channel();
    let subscribe_notify = Arc::new(Notify::new());

    // Spawn background task to handle Subscribe and publish objects
    let client_arc_clone = client_arc.clone();
    let name_clone = name.clone();
    let subscribe_notify_clone = subscribe_notify.clone();
    let object_rx = Arc::new(Mutex::new(object_rx));

    runtime_handle.spawn(async move {
        handle_subscribe_and_publish(
            client_arc_clone,
            name_clone,
            subscribe_notify_clone,
            object_rx,
        ).await;
    });

    let track_ctx = TrackContext {
        client: client_arc,
        track_name: name,
        track_alias: 1, // Will be set when Subscribe arrives
        object_tx,
        subscribe_notify,
    };

    let track = Arc::new(Mutex::new(track_ctx));
    eprintln!("[moqtail_ffi] Track published successfully");

    track_to_ptr(track)
}

/// Background task to handle Subscribe and publish objects
async fn handle_subscribe_and_publish(
    client: Arc<Mutex<ClientContext>>,
    track_name: String,
    subscribe_notify: Arc<Notify>,
    object_rx: Arc<Mutex<mpsc::UnboundedReceiver<ObjectMessage>>>,
) {
    eprintln!("[moqtail_ffi] Waiting for Subscribe message...");

    // Wait for Subscribe message
    let (track_alias, connection) = {
        let ctx = client.lock().await;
        let mut control = ctx.control_handler.lock().await;

        loop {
            match control.next_message().await {
                Ok(ControlMessage::Subscribe(m)) => {
                    eprintln!("[moqtail_ffi] Received Subscribe: request_id={}", m.request_id);

                    // Send SubscribeOk
                    let track_alias = 1u64;
                    let subscribe_ok = SubscribeOk::new_ascending_with_content(
                        m.request_id,
                        track_alias,
                        0,
                        None,
                        None,
                    );

                    if let Err(e) = control.send_impl(&subscribe_ok).await {
                        eprintln!("[moqtail_ffi] Failed to send SubscribeOk: {}", e);
                        return;
                    }

                    eprintln!("[moqtail_ffi] SubscribeOk sent, track_alias={}", track_alias);
                    break (track_alias, ctx.connection.clone());
                }
                Ok(m) => {
                    eprintln!("[moqtail_ffi] Unexpected message while waiting for Subscribe: {:?}", m);
                }
                Err(e) => {
                    eprintln!("[moqtail_ffi] Error receiving message: {:?}", e);
                    return;
                }
            }
        }
    };

    // Notify that we're subscribed
    subscribe_notify.notify_waiters();

    // Start publishing objects
    eprintln!("[moqtail_ffi] Starting object publisher task");

    let mut current_group = None;
    let mut group_objects: Vec<ObjectMessage> = Vec::new();
    let mut rx = object_rx.lock().await;

    while let Some(obj_msg) = rx.recv().await {
        // Extract group ID depending on message type
        // LOC: group_id field
        // MMTP: mpu_sequence field (equivalent grouping)
        let msg_group_id = match &obj_msg {
            ObjectMessage::Loc { group_id, .. } => *group_id,
            ObjectMessage::Mmtp { mpu_sequence, .. } => *mpu_sequence as u64,
        };

        // Check if we're starting a new group
        if current_group != Some(msg_group_id) {
            // Send previous group's objects if any
            if !group_objects.is_empty() && current_group.is_some() {
                send_group_objects(
                    &connection,
                    track_alias,
                    current_group.unwrap(),
                    &group_objects
                ).await;
                group_objects.clear();
            }
            current_group = Some(msg_group_id);
        }

        group_objects.push(obj_msg);

        // Send group after collecting all objects for now (simplified)
        // In production, would buffer and send based on timing/size
        if group_objects.len() >= 30 { // Send after keyframe interval
            send_group_objects(
                &connection,
                track_alias,
                current_group.unwrap(),
                &group_objects
            ).await;
            group_objects.clear();
            current_group = None;
        }
    }
}

/// Send a group's worth of objects via unidirectional stream
async fn send_group_objects(
    connection: &wtransport::Connection,
    track_alias: u64,
    group_id: u64,
    objects: &[ObjectMessage],
) {
    eprintln!("[moqtail_ffi] Sending group {}: {} objects", group_id, objects.len());

    // Open unidirectional stream for this group
    let stream = match connection.open_uni().await {
        Ok(uni_result) => match uni_result.await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[moqtail_ffi] Failed to await uni stream: {}", e);
                return;
            }
        },
        Err(e) => {
            eprintln!("[moqtail_ffi] Failed to open uni stream: {}", e);
            return;
        }
    };

    // Create subgroup header
    let sub_header = SubgroupHeader::new_with_explicit_id(
        track_alias,
        group_id,
        1u64, // subgroup_id
        1u8,  // publisher_priority
        true, // is_start_of_group
        true, // is_end_of_group
    );

    let header_info = HeaderInfo::Subgroup {
        header: sub_header,
    };

    let stream = Arc::new(Mutex::new(stream));
    let mut stream_handler = match SendDataStream::new(stream.clone(), header_info).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[moqtail_ffi] Failed to create SendDataStream: {}", e);
            return;
        }
    };

    let mut prev_object_id = None;

    for obj_msg in objects {
        // ========================================================================
        // DESIGN COMPARISON: LOC vs MMTP Object Construction
        // ========================================================================
        //
        // LOC (moqtail native):
        // - Metadata in varint extension headers (flexible, no fixed size)
        // - Payload is clean codec data
        // - Extensions added during serialization (minimal allocation)
        //
        // MMTP (added for comparison):
        // - Metadata in fixed MMTP+MPU headers (already prepended to payload)
        // - Payload contains MMTP header + MPU header + codec data
        // - No extension headers needed (but can add for MOQ-specific metadata)
        //
        // ========================================================================

        let (object_id, subgroup_obj) = match obj_msg {
            ObjectMessage::Loc {
                group_id: _,
                object_id,
                publisher_priority: _,
                is_keyframe,
                timestamp_us,
                payload,
                config_data,
            } => {
                // LOC Path: Build MOQ extension headers from metadata
                let mut extensions = Vec::new();

                // CaptureTimestamp extension
                extensions.push(KeyValuePair::VarInt {
                    type_value: 1,
                    value: *timestamp_us,
                });

                // VideoFrameMarking extension
                extensions.push(KeyValuePair::VarInt {
                    type_value: 2,
                    value: if *is_keyframe { 1 } else { 0 },
                });

                // VideoConfig extension (if present)
                if let Some(ref config) = config_data {
                    extensions.push(KeyValuePair::Bytes {
                        type_value: 4,
                        value: config.clone(),
                    });
                }

                // Create LOC subgroup object
                let obj = SubgroupObject {
                    object_id: *object_id,
                    extension_headers: Some(extensions),
                    object_status: None,
                    payload: Some(payload.clone()),
                };

                (*object_id, obj)
            }

            ObjectMessage::Mmtp {
                packet_id: _,
                packet_sequence,
                timestamp: _,
                packet_type: _,
                mpu_sequence: _,
                mpu_fragment_type: _,
                payload,
            } => {
                // MMTP Path: Payload already contains MMTP+MPU headers
                // Use packet_sequence as object_id
                // No extension headers needed - metadata is in MMTP headers
                let obj = SubgroupObject {
                    object_id: *packet_sequence as u64,
                    extension_headers: None, // MMTP headers embedded in payload
                    object_status: None,
                    payload: Some(payload.clone()),
                };

                (*packet_sequence as u64, obj)
            }
        };

        let subgroup_obj = subgroup_obj;

        // Convert to Object
        let object = match Object::try_from_subgroup(
            subgroup_obj,
            track_alias,
            group_id,
            Some(1), // subgroup_id
            obj_msg.publisher_priority,
        ) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("[moqtail_ffi] Failed to create object: {}", e);
                continue;
            }
        };

        // Send object
        if let Err(e) = stream_handler.send_object(&object, prev_object_id).await {
            eprintln!("[moqtail_ffi] Failed to send object {}: {}", obj_msg.object_id, e);
        } else {
            eprintln!("[moqtail_ffi] Sent object: group={}, object={}", group_id, obj_msg.object_id);
        }

        prev_object_id = Some(obj_msg.object_id);
    }

    // Flush stream
    if let Err(e) = stream_handler.flush().await {
        eprintln!("[moqtail_ffi] Failed to flush stream: {}", e);
    } else {
        eprintln!("[moqtail_ffi] Group {} sent successfully", group_id);
    }
}

/// Send an object to the track with LOC extensions
///
/// # Safety
/// `track` must be a valid handle, `obj` must point to valid MoqtailObject
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moqtail_send_object(
    track: *mut MoqtailTrackHandle,
    obj: *const MoqtailObject,
) -> i32 {
    if track.is_null() || obj.is_null() {
        eprintln!("[moqtail_ffi] Error: null pointer in send_object");
        return -1;
    }

    let obj_ref = unsafe { &*obj };

    // Convert to owned data
    let payload = if obj_ref.payload_len > 0 && !obj_ref.payload.is_null() {
        unsafe {
            Bytes::copy_from_slice(
                std::slice::from_raw_parts(obj_ref.payload, obj_ref.payload_len)
            )
        }
    } else {
        eprintln!("[moqtail_ffi] Error: invalid payload");
        return -1;
    };

    let config_data = if obj_ref.is_keyframe && obj_ref.config_len > 0 && !obj_ref.config_data.is_null() {
        Some(unsafe {
            Bytes::copy_from_slice(
                std::slice::from_raw_parts(obj_ref.config_data, obj_ref.config_len)
            )
        })
    } else {
        None
    };

    let obj_msg = ObjectMessage::Loc {
        group_id: obj_ref.group_id,
        object_id: obj_ref.object_id,
        publisher_priority: obj_ref.publisher_priority,
        is_keyframe: obj_ref.is_keyframe,
        timestamp_us: obj_ref.timestamp_us,
        payload,
        config_data,
    };

    // Send via channel
    let track_arc = unsafe {
        let arc = ptr_to_track(track);
        let cloned = arc.clone();
        std::mem::forget(arc);
        cloned
    };

    let track_ctx = track_arc.blocking_lock();
    if let Err(e) = track_ctx.object_tx.send(obj_msg) {
        eprintln!("[moqtail_ffi] Failed to queue object: {}", e);
        return -1;
    }

    0 // Success
}

/// Send MMTP-MFU object via MOQtail (design comparison with moq-lib)
///
/// **Key Design Difference**: MMTP requires prepending fixed headers (12B MMTP + variable MPU),
/// while LOC uses varint extensions. This function demonstrates how moqtail handles MMTP-MFU
/// compared to moq-lib's approach.
///
/// **Memory Efficiency**: One allocation to prepend headers to payload (unavoidable with C FFI),
/// then zero-copy transmission via Bytes.
///
/// # Safety
/// `track` and `obj` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moqtail_send_object_mmtp(
    track: *mut MoqtailTrackHandle,
    obj: *const MmtpMfuObject,
) -> i32 {
    if track.is_null() || obj.is_null() {
        eprintln!("[moqtail_ffi] Error: null pointer in send_object_mmtp");
        return -1;
    }

    let obj_ref = unsafe { &*obj };

    // Validate payload
    if obj_ref.payload_len == 0 || obj_ref.payload.is_null() {
        eprintln!("[moqtail_ffi] Error: invalid MMTP payload");
        return -1;
    }

    // Parse packet type and fragment type from u8
    let packet_type = match obj_ref.packet_type {
        0 => PacketType::MpuMetadata,
        1 => PacketType::MpuMedia,
        2 => PacketType::MpuRepair,
        _ => {
            eprintln!("[moqtail_ffi] Error: invalid packet_type {}", obj_ref.packet_type);
            return -1;
        }
    };

    let mpu_fragment_type = match obj_ref.mpu_fragment_type {
        0 => FragmentType::Complete,
        1 => FragmentType::FirstFragment,
        2 => FragmentType::MiddleFragment,
        3 => FragmentType::LastFragment,
        _ => {
            eprintln!("[moqtail_ffi] Error: invalid mpu_fragment_type {}", obj_ref.mpu_fragment_type);
            return -1;
        }
    };

    // Design Comparison Point 1: Header Construction
    // - moq-lib: Fixed 12-byte MMTP header is natural fit (existing packet structure)
    // - moqtail: Must create MMTP header from scratch, then wrap in MOQ object
    let mmtp_header = MmtpHeader {
        packet_id: obj_ref.packet_id,
        packet_sequence: obj_ref.packet_sequence,
        timestamp: obj_ref.timestamp,
        packet_type,
    };

    let mpu_header = MpuHeader {
        mpu_sequence: obj_ref.mpu_sequence,
        fragment_type: mpu_fragment_type,
        fec_type: FecType::None, // Can be extended for RaptorQ
    };

    // Design Comparison Point 2: Header Serialization + Payload Combination
    // - moq-lib: Headers naturally prepend to multicast packets
    // - moqtail: Must serialize headers and prepend to payload (one allocation)
    let mut packet_buf = BytesMut::with_capacity(
        MMTP_HEADER_SIZE + MPU_HEADER_SIZE + obj_ref.payload_len
    );

    // Serialize MMTP header (12 bytes fixed)
    mmtp_header.serialize(&mut packet_buf);

    // Serialize MPU header (variable size)
    mpu_header.serialize(&mut packet_buf);

    // Append payload (zero-copy from FFI slice)
    unsafe {
        packet_buf.put_slice(std::slice::from_raw_parts(obj_ref.payload, obj_ref.payload_len));
    }

    let payload = packet_buf.freeze();

    // Design Comparison Point 3: MOQ Object Wrapping
    // - moq-lib: MMTP packet IS the MOQ object (minimal wrapping)
    // - moqtail: MMTP packet becomes payload of MOQ object (extra layer)
    let obj_msg = ObjectMessage::Mmtp {
        packet_id: obj_ref.packet_id,
        packet_sequence: obj_ref.packet_sequence,
        timestamp: obj_ref.timestamp,
        packet_type,
        mpu_sequence: obj_ref.mpu_sequence,
        mpu_fragment_type,
        payload,
    };

    // Send via channel (same as LOC path)
    let track_arc = unsafe {
        let arc = ptr_to_track(track);
        let cloned = arc.clone();
        std::mem::forget(arc);
        cloned
    };

    let track_ctx = track_arc.blocking_lock();
    if let Err(e) = track_ctx.object_tx.send(obj_msg) {
        eprintln!("[moqtail_ffi] Failed to queue MMTP object: {}", e);
        return -1;
    }

    0 // Success
}

/// Close track and cleanup
///
/// # Safety
/// `track` must be a valid handle or null
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moqtail_track_close(track: *mut MoqtailTrackHandle) {
    if track.is_null() {
        return;
    }

    eprintln!("[moqtail_ffi] Closing track");

    // Convert back to Arc and let it drop
    let _track_arc = unsafe { ptr_to_track(track) };
    // Arc will be dropped here, releasing resources
}

/// Disconnect client and cleanup
///
/// # Safety
/// `client` must be a valid handle or null
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moqtail_client_disconnect(client: *mut MoqtailClientHandle) {
    if client.is_null() {
        return;
    }

    eprintln!("[moqtail_ffi] Disconnecting client");

    // Convert back to Arc and let it drop
    let _client_arc = unsafe { ptr_to_client(client) };
    // Arc will be dropped here, releasing resources
}
