import unittest
import sys
import os
import uuid
import time
import queue

# Ensure we can find the driver in the parent directory
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from driver import MoqClient

RELAY_URL = "https://localhost:4433" 

class TestTrackStatus(unittest.TestCase):
    def setUp(self):
        self.client = MoqClient()
        if not self.client.start():
            self.fail("Failed to start Rust binary")
        self.client.connect(RELAY_URL)
        
        # We publish a unique namespace so the Relay knows to route requests to US
        self.namespace = f"status-test-{uuid.uuid4().hex[:8]}"
        self.client.publish_namespace(self.namespace)

    def tearDown(self):
        self.client.stop()

    def test_track_status_flow(self):
        """
        Test the full Track Status Loop:
        1. Client sends 'get_track_status' RPC.
        2. Rust sends 'TRACK_STATUS' to Relay.
        3. Relay forwards to Publisher (ourself).
        4. Rust Publisher replies 'TRACK_STATUS_OK'.
        5. Relay forwards back to Client.
        6. Rust Client emits 'on_track_status' event.
        """
        track_name = "my-status-track"
        print(f"\n--- Test: Track Status Loop ({self.namespace}/{track_name}) ---")
        
        # 1. Send the Request
        # We rely on the Relay to route this "Loopback" since we own the namespace.
        print("Sending Track Status Request...")
        res = self.client.send_command("track_status", {
            "namespace": self.namespace,
            "track": track_name
        })
        
        print(f"RPC Response: {res}")
        self.assertEqual(res.get("status"), "sent")

        # 2. Wait for the Response Event
        # The Rust client emits 'on_track_status' when it receives the OK message.
        print("Waiting for on_track_status event...")
        evt = self.client.wait_for_event("on_track_status", timeout=5)
        
        self.assertIsNotNone(evt, "Timed out waiting for Track Status response. Relay might not be forwarding it.")
        
        # ... inside test_track_status_flow ...

        # ... inside test_track_status_flow ...

        params = evt["params"]
        print(f"Received Status: {params}")
        
        # Normalize Namespace (handle / prefix)
        received_ns = params["namespace"].lstrip('/')
        expected_ns = self.namespace.lstrip('/')
        self.assertEqual(received_ns, expected_ns)
        
        self.assertEqual(params["track"], track_name)

        # CORRECT ASSERTIONS:
        # 1. We expect content to exist (since we mocked it)
        # Note: 'content_exists' might be boolean true/false or string "true"/"false" depending on serde
        self.assertTrue(params.get("content_exists") in [True, "true", 1], "Expected Content Exists to be True")
        
        
        # 3. Check Object ID (0)
        self.assertEqual(params["last_object"], 0)

        print("Success: Track Status loop completed successfully.")

if __name__ == '__main__':
    unittest.main()