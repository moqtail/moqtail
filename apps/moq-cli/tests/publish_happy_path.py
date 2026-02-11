import unittest
import sys
import os
import time

# Add parent dir to path so we can import driver
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from driver import MoqClient

# Configuration
RELAY_URL = "https://localhost:4433"
NAMESPACE = "test-interaction"
TRACK = "push-track"

class TestPublishHappyPath(unittest.TestCase):
    def setUp(self):
        # We need two distinct clients
        self.publisher = MoqClient()
        self.subscriber = MoqClient()
        
        if not self.publisher.start():
            self.fail("Failed to start Publisher binary")
        if not self.subscriber.start():
            self.publisher.stop()
            self.fail("Failed to start Subscriber binary")

    def tearDown(self):
        self.publisher.stop()
        self.subscriber.stop()

    def test_publish_flow(self):
        """
        Verifies that a Publisher can push a track to a Subscriber 
        who has expressed interest via SubscribeNamespace.
        """
        print(f"\n--- Test: Publish Happy Path (Push with Discovery) ---")
        
        # 1. Connect Subscriber
        print(f"[Subscriber] Connecting to {RELAY_URL}...")
        res_sub = self.subscriber.connect(RELAY_URL)
        self.assertEqual(res_sub.get("status"), "connected")

        # 2. Subscriber signals interest (The "Mailbox")
        # Without this, the Relay would drop the incoming Publish message.
        print(f"[Subscriber] Subscribing to namespace prefix '{NAMESPACE}'...")
        res_ns = self.subscriber.subscribe_namespace(namespace_prefix=NAMESPACE)
        self.assertEqual(res_ns.get("status"), "subscribed_to_namespace")
        print("[Subscriber] Ready to receive updates.")

        # 3. Connect Publisher
        print(f"[Publisher] Connecting to {RELAY_URL}...")
        res_pub = self.publisher.connect(RELAY_URL)
        self.assertEqual(res_pub.get("status"), "connected")

        # 4. Publisher sends PUBLISH (The "Push")
        print(f"[Publisher] Sending PUBLISH for {NAMESPACE}/{TRACK}...")
        pub_res = self.publisher.publish(
            namespace=NAMESPACE,
            track=TRACK,
            start_group=0,
            start_object=0
        )
        self.assertEqual(pub_res.get("status"), "publishing")
        print("[Publisher] Publish command accepted. Blasting data...")

        # 5. Verification: Control Plane
        # The Relay should see the match and forward the PUBLISH message to the Subscriber.
        print("[Subscriber] Waiting for 'on_peer_publish' control message...")
        peer_event = self.subscriber.wait_for_event("on_peer_publish", timeout=5)
        
        if peer_event:
            print(f"[Subscriber] SUCCESS: Received Push Notification! {peer_event}")
            params = peer_event.get("params", {})
            received_ns = params.get("namespace", "").lstrip('/')
            self.assertEqual(received_ns, NAMESPACE)
            self.assertEqual(params.get("track"), TRACK)
        else:
            self.fail("[Subscriber] FAIL: Did not receive 'on_peer_publish'. Relay forwarding broken.")

        # 6. Verification: Data Plane
        # The Subscriber should automatically start receiving object stats.
        print("[Subscriber] Waiting for data flow ('on_stat_update')...")
        stat_event = self.subscriber.wait_for_event("on_stat_update", timeout=5)
        
        if stat_event:
            print(f"[Subscriber] SUCCESS: Data is flowing! {stat_event}")
        else:
            self.fail("[Subscriber] FAIL: Did not receive data stats. Subscription wiring broken.")

if __name__ == '__main__':
    unittest.main()