import unittest
import sys
import os
import time

sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from driver import MoqClient

RELAY_URL = "https://localhost:4433" 

class TestFullWorkflowTDD(unittest.TestCase):
    def setUp(self):
        # Create TWO clients
        self.publisher = MoqClient()
        self.subscriber = MoqClient()
        
        if not self.publisher.start():
            self.fail("Failed to start Publisher")
        if not self.subscriber.start():
            self.publisher.stop()
            self.fail("Failed to start Subscriber")

    def tearDown(self):
        self.publisher.stop()
        self.subscriber.stop()

    def test_01_namespace_discovery(self):
        """
        TDD: Test Namespace Discovery Workflow.
        1. Announce 'meet/room1' (Use Slash!)
        2. SubscribeNamespace 'meet'
        3. Expect 'on_peer_namespace' notification with 'meet/room1'
        """
        print("\n--- TDD: Namespace Discovery ---")
        self.publisher.connect(RELAY_URL)
        self.subscriber.connect(RELAY_URL)
        time.sleep(0.5)

        # 1. Announce (Use correct path separator)
        print("[Publisher] Announcing 'meet/room1'")
        self.publisher.publish_namespace("meet/room1") 
        time.sleep(0.5)
        
        # 2. Subscribe (Prefix match works on path segments)
        print("[Subscriber] Subscribing to prefix 'meet'")
        self.subscriber.subscribe_namespace("meet")

        # 3. Wait for the NAMESPACE update
        print("Waiting for 'on_peer_namespace' notification...")
        event = self.subscriber.wait_for_event("on_peer_namespace", timeout=5)
        
        if event:
            print(f"SUCCESS: Received Namespace Update: {event}")
            params = event.get("params", {})
            # Handle the leading slash that to_utf8_path() might add
            received = params.get("matched_namespace", "").lstrip('/')
            self.assertEqual(received, "meet/room1")
        else:
            self.fail("FAIL: Subscriber did not receive 'on_peer_namespace'.")
    
    def test_02_publish_lifecycle(self):
        """
        TDD: Test Publish Lifecycle (Push) with TWO clients.
        """
        print("\n--- TDD: Publish Lifecycle ---")
        self.publisher.connect(RELAY_URL)
        self.subscriber.connect(RELAY_URL)

        # 1. Subscriber Interest
        self.subscriber.subscribe_namespace("live")

        # 2. Publisher Pushes
        self.publisher.publish("live", "obs-stream")

        # 3. Verify Subscriber gets it
        print("Waiting for 'on_peer_publish'...")
        event = self.subscriber.wait_for_event("on_peer_publish", timeout=3)
        
        if event:
            print(f"SUCCESS: Received Peer Publish: {event}")
        else:
            self.fail("FAIL: Subscriber did not receive Push.")

if __name__ == '__main__':
    unittest.main()