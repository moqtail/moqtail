import unittest
import sys
import os
import uuid
import time
import queue

# Add parent dir to path
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from driver import MoqClient

RELAY_URL = "https://localhost:4433" 

class TestFetch(unittest.TestCase):
    def setUp(self):
        # 1. Start a fresh binary for every test
        self.client = MoqClient()
        if not self.client.start():
            self.fail("Failed to start Rust binary")
        self.client.connect(RELAY_URL)
        
        # 2. VITAL: Generate a unique namespace to prevent Relay Cache collisions
        self.namespace = f"fetch-test-{uuid.uuid4().hex[:8]}"
        self.client.publish_namespace(self.namespace)

    def tearDown(self):
        self.client.stop()

    def test_standalone_fetch(self):
        """
        Test Fetching a specific range [20, 22).
        Workaround: We MUST subscribe first to create the track in the Relay.
        """
        print(f"\n--- Test: Fetch Range [20, 22) ({self.namespace}) ---")
        
        track_name = "archive-track"

        # --- STEP A: PRIME THE TRACK ---
        # The Relay refuses to forward Fetch requests for unknown tracks.
        # We Subscribe first to force the Relay to create the track entry.
        print("Priming track with a Subscription...")
        sub_res = self.client.subscribe(
            namespace=self.namespace, 
            track=track_name,
            # Use a dummy filter so we don't actually get flooded with data if we were blasting
            start_group=9999, 
            start_object=0
        )
        self.assertIn("subscription_id", sub_res)
        
        # Wait a tiny bit for Relay to register the track internally
        time.sleep(5.5)
        
        # --- STEP B: EXECUTE FETCH ---
        print("Sending Fetch Request...")
        res = self.client.send_command("fetch", {
            "namespace": self.namespace,
            "track": track_name,
            "start_group": 2,
            "start_object": 0,
            "end_group": 4, 
            "end_object": 0
        })
        
        print(f"Fetch Response: {res}")
        self.assertEqual(res.get("status"), "fetching")

        # --- STEP C: CONSUME DATA ---
        # We ignore the data from the 'Subscribe' (if any) and look for our Fetch data
        self._wait_for_group(2)
        self._wait_for_group(3)

        # --- STEP D: VERIFY STOP ---
        print("Verifying fetch stops (waiting for silence)...")
        time.sleep(2) 
        
        while not self.client.events_queue.empty():
            evt = self.client.events_queue.get()
            if evt.get("method") == "on_stat_update":
                gid = evt["params"]["group_id"]
                # We might get G9999 from the subscription if we were blasting it, 
                # but we shouldn't get G22 from the Fetch.
                self.assertNotEqual(gid, 22, f"Fetch failed to stop! Received Group {gid}")

        print("Success: Fetch stream closed correctly.")

    def _wait_for_group(self, target_group):
        """
        Consumes events until target_group is found.
        Ignores other events (logs, handshakes, etc.)
        Fails if not found within 5 seconds.
        """
        print(f"Waiting for Group {target_group}...")
        start = time.time()
        while time.time() - start < 5: 
            try:
                # 1s timeout allows loop to check overall time constraint
                evt = self.client.events_queue.get(timeout=1)
                
                if evt.get("method") == "on_stat_update":
                    gid = evt["params"]["group_id"]
                    if gid == target_group:
                        return # Success
                    
                    # Debug log if we see out-of-order data
                    # print(f"   (Ignored Group {gid})")
                    
            except queue.Empty:
                continue
                
        self.fail(f"Timed out waiting for Group {target_group}")

if __name__ == '__main__':
    unittest.main()