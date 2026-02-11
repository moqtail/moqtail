import sys
import os
import time
import signal
import uuid

# Ensure we can import the driver
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from driver import MoqClient

RELAY_URL = "https://localhost:4433" 
NAMESPACE = f"moq-date" # Random ID to avoid collision
TRACK_NAME = "chill-track"

def main():
    client = MoqClient()
    
    print(f"🚀 Starting MoQ Client...")
    if not client.start():
        print("❌ Failed to start Rust binary")
        sys.exit(1)

    try:
        print(f"🔗 Connecting to {RELAY_URL}...")
        client.connect(RELAY_URL)
        
        # 1. Announce the Namespace
        print(f"📢 Announcing Namespace: '{NAMESPACE}'")
        client.publish_namespace(NAMESPACE)
        
        # 2. (Optional) You can also 'Announce' a track if your logic requires it,
        # but usually announcing the namespace is enough for the Relay to route to you.
        print(f"✨ Ready! holding namespace open.")
        print(f"   Test against this using:")
        print(f"   Namespace: {NAMESPACE}")
        print(f"   Track:     {TRACK_NAME}")
        print("zzz... (Press Ctrl+C to stop)")

        # 3. Loop forever and print logs
        while True:
            # Drain the queue so we see what's happening (e.g. incoming subscribes)
            if not client.events_queue.empty():
                evt = client.events_queue.get()
                method = evt.get("method")
                
                # Filter out boring noise if you want
                if method == "log":
                    print(f"[LOG] {evt['params'].get('message')}")
                elif method == "on_peer_subscribe":
                    print(f"🔔 NEW SUBSCRIBER! ID: {evt['params'].get('subscribe_id')} for {evt['params'].get('track_name')}")
                elif method == "on_track_status":
                     print(f"❓ Track Status Requested")
                else:
                    print(f"[EVENT] {evt}")
            
            time.sleep(0.1)

    except KeyboardInterrupt:
        print("\n🛑 Ctrl+C received. Stopping...")
    finally:
        client.stop()
        print("👋 Bye.")

if __name__ == '__main__':
    main()