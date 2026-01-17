import unittest
import sys
import os
import uuid
import time

sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from driver import MoqClient

RELAY_URL = "https://localhost:4433" 

class TestSubscriptionFilters(unittest.TestCase):
    def setUp(self):
        self.client = MoqClient()
        if not self.client.start():
            self.fail("Failed to start Rust binary")
        self.client.connect(RELAY_URL)
        
        # 1. Generate UNIQUE namespace for this specific test method run
        self.namespace = f"test-ns-{uuid.uuid4().hex[:8]}"
        self.client.publish_namespace(self.namespace)

    def tearDown(self):
        self.client.stop()

    def test_absolute_start_filter(self):
        print(f"\n--- Test: Absolute Start ({self.namespace}) ---")
        
        # 2. USE self.namespace (Do not use "filter-test-ns"!)
        sub_res = self.client.subscribe(
            namespace=self.namespace, 
            track="track-1",
            filter_type="absolute_start",
            start_group=5,
            start_object=0
        )
        
        print("Waiting for data (timeout 10s)...")
        # Increase timeout slightly to be safe against startup jitter
        first_stat = self.client.wait_for_event("on_stat_update", timeout=10)
        self.assertIsNotNone(first_stat, "No data received")
        
        received_group = first_stat["params"]["group_id"]
        self.assertGreaterEqual(received_group, 5, 
            f"Expected Group >= 5, but got {received_group}")

    def test_absolute_range_filter(self):
        print(f"\n--- Test: Absolute Range ({self.namespace}) ---")
        
        # 3. USE self.namespace
        sub_res = self.client.subscribe(
            namespace=self.namespace, 
            track="track-1",
            filter_type="absolute_range",
            start_group=10,
            start_object=0,
            end_group=12 
        )
        
        self._wait_for_group(10)
        self._wait_for_group(11)
        self._wait_for_group(12)
        
        time.sleep(2)
        # Check for leakage
        while not self.client.events_queue.empty():
            evt = self.client.events_queue.get()
            if evt.get("method") == "on_stat_update":
                gid = evt["params"]["group_id"]
                self.assertLessEqual(gid, 12, f"Stream failed to stop! Got {gid}")

    def _wait_for_group(self, target_group):
        import queue
        start = time.time()
        while time.time() - start < 5: 
            try:
                evt = self.client.events_queue.get(timeout=1)
                if evt.get("method") == "on_stat_update":
                    if evt["params"]["group_id"] == target_group:
                        return
            except queue.Empty:
                continue
        self.fail(f"Timed out waiting for Group {target_group}")

if __name__ == '__main__':
    unittest.main()