---
description: How an MSF catalog describes a broadcast, how LOC, CMAF and LOCMAF package media into objects, and what an MSF URL points at.
---

# Catalogs and packaging

MOQT moves objects but never reads them. Two more layers make those objects playable: a catalog, which says what tracks a broadcast has and how to decode them, and a packaging, which says what each object's payload holds. MOQtail follows the MOQT Streaming Format (MSF) for catalogs and plays three packagings: LOC, CMAF and LOCMAF.

## The catalog

An MSF catalog is a JSON document published on a track of its own, usually named `catalog`, in the broadcast's namespace. It lists the broadcast's tracks with what a player needs to know before it subscribes to any of them:

```json
{
  "version": "draft-02",
  "tracks": [
    {
      "name": "video-1080p",
      "packaging": "cmaf",
      "role": "video",
      "isLive": true,
      "altGroup": 1,
      "codec": "avc1.640028",
      "bitrate": 6000000,
      "width": 1920,
      "height": 1080,
      "framerate": 30
    },
    {
      "name": "video-480p",
      "packaging": "cmaf",
      "role": "video",
      "isLive": true,
      "altGroup": 1,
      "codec": "avc1.64001f",
      "bitrate": 1500000,
      "width": 854,
      "height": 480,
      "framerate": 30
    },
    {
      "name": "audio",
      "packaging": "cmaf",
      "role": "audio",
      "isLive": true,
      "codec": "mp4a.40.2",
      "samplerate": 48000,
      "channelConfig": "2",
      "lang": "en"
    }
  ]
}
```

A track without a `namespace` is in the catalog's own namespace. Besides the media details, a track can carry timing hints the player acts on: `targetLatency` or `buffers` for the latency to hold, `trackDuration` for on-demand content, and `accessibleWindowTime` for how far back a live stream can be seeked.

The catalog track is a track like any other, so it changes over time. Object 0 of each group holds a whole catalog, and the later objects of the group hold delta updates that add, remove, clone or update tracks. Payloads may be gzip-compressed, and string values may contain variables that the player fills in from the URL.

### What the player reads from it

**Switching sets.** Tracks of the same content at different qualities form a switching set, and each track in it is a rendition. The player takes the tracks of the first `renderGroup`, and the tracks that share an `altGroup` and a `lang` form one set. A track without an `altGroup` is a set of its own. A track that `depends` on another can't be decoded alone, so it is never picked. In the catalog above, the two video tracks are one set and the audio track is another.

**One set per role.** The player plays one video set and one audio set. For audio it prefers the language you ask for, or else the browser's. Within a set it starts on the track with the smallest `keyframeSpacing`, because that one switches soonest, and switches among the renditions as bandwidth changes.

**Init data.** A decoder needs its configuration before the first frame: for CMAF the CMAF Header (the init segment), for LOC the codec configuration bytes. A track's `initRef` names an entry in the catalog's `initDataList`, which holds the bytes inline or says which track or object property carries them.

**Timelines.** A media timeline track maps media time to locations and wallclock time, which the player uses to seek and to measure latency. Event timelines carry timed records. The built-in one is the SAP timeline of CMSF, which gives each object's stream access point type and earliest presentation time.

## Packaging

The `packaging` field of a track says what its objects hold.

**LOC**, the Low Overhead Media Container, puts one encoded frame in each object, with no container around it. What a decoder needs rides in LOC properties next to the payload: a timestamp, an optional timescale, the codec string and the decoder configuration. The frames go straight to WebCodecs. Without a timescale, timestamps are microseconds since the Unix epoch, so audio and video share one clock, which the `<canvas>` sink uses to keep them in sync.

**CMAF**, as the CMAF Streaming Format (CMSF) uses it, puts a CMAF chunk in each object: a `moof` box and its `mdat`. The init segment comes from the catalog or a property. These are the bytes Media Source Extensions (MSE) take, so a CMAF track plays in a `<video>` with no repackaging, live or on demand.

**LOCMAF** is CMAF with less overhead. Each object replaces the `moof` with a compact header of field values, which the receiver completes from the defaults in the CMAF Header, and later objects in a group can carry only what changed. The player rebuilds the full CMAF chunk from it, so LOCMAF plays wherever CMAF does. When a catalog offers the same media as both, the player prefers LOCMAF.

### Where each one plays

The player has two sinks, and each takes different packagings:

| Sink       | Browser API                                       | Packaging    | Plays                       |
| ---------- | ------------------------------------------------- | ------------ | --------------------------- |
| `<video>`  | MSE, through `ManagedMediaSource` where it exists | CMAF, LOCMAF | Live and on demand, seeking |
| `<canvas>` | WebCodecs, with audio through an AudioWorklet     | LOC          | Live only, no seeking back  |

The sink decides which switching sets the player picks. When a catalog offers the same content in both LOC and CMAF, `.sink(video)` plays the CMAF tracks and `.sink(canvas)` plays the LOC ones. A rendition the browser can't decode, as MSE or WebCodecs reports it, leaves its set, so the player never switches to it. [How the player works](/guide/concepts/player) explains how it chooses.

## MSF URLs

An MSF URL names a session and a track in it. The player takes one in `source()`, and `<moqtail-video>` takes one in `src`:

```
moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog
└───────── session ────┘     └─namespace┘  └track┘
```

The part before the fragment is the `moqt://` URL of the session. After `#msf:` come the namespace fields joined by `-`, then `--`, then the track name. So this URL names the track `catalog` in the namespace (`moqtail`, `ch07`). Letters, digits and `_` stand for themselves; every other byte is written as `.` and two hex digits, so names keep their exact bytes.

The track is usually the catalog, which the built-in elements start from. Parameters after the track, such as `&location-range=34-64`, ask for part of a track by location, media time or wallclock time, and every parameter is also a variable for the catalog. [MSF URLs](/ts/reference/msf-url) has the full grammar.

## Related

- [How the player works](/guide/concepts/player)
- [MSF URLs](/ts/reference/msf-url)
- [The drafts MOQtail supports](/guide/#supported-drafts)
