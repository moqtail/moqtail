# tests/test_basic.py
import unittest
import time
import sys
import os

# Add parent dir to path so we can import driver
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from driver import MoqClient

# Configuration
RELAY_URL = "https://localhost:4433" 

class TestMoqBasic(unittest.TestCase):
    def setUp(self):
        self.client = MoqClient()
        if not self.client.start():
            self.fail("Failed to start Rust binary")

    def tearDown(self):
        self.client.stop()

    def test_01_connect(self):
        """Test that we can connect to the relay."""
        print("\n--- Test 01: Connect ---")
        res = self.client.connect(RELAY_URL)
        print(f"Connect Response: {res}")
        self.assertEqual(res.get("status"), "connected")

    def test_02_publish_namespace(self):
        """Test announcing a namespace (Draft-16: PublishNamespace)."""
        print("\n--- Test 02: Publish Namespace ---")
        self.client.connect(RELAY_URL)
        
        res = self.client.publish_namespace("test-integration-ns")
        print(f"Publish Result: {res}")
        self.assertEqual(res.get("status"), "namespace_published")

    def test_03_loopback_subscription(self):
        """
        Test the full loop: 
        1. Connect
        2. Publish Namespace
        3. Subscribe to ourselves
        4. Verify we receive 'on_peer_subscribe' (as publisher)
        5. Verify we receive 'on_stat_update' (as subscriber)
        """
        print("\n--- Test 03: Loopback Subscription ---")
        self.client.connect(RELAY_URL)
        self.client.publish_namespace("loopback-ns")
        
        # Subscribe to our own namespace
        sub_res = self.client.subscribe("loopback-ns", "track1")
        self.assertIn("subscription_id", sub_res)
        sub_id = sub_res["subscription_id"]
        print(f"Subscribed with ID: {sub_id}")

        # 1. As the Publisher, we should get notified that someone (ourselves) subscribed
        print("Waiting for 'on_peer_subscribe'...")
        peer_event = self.client.wait_for_event("on_peer_subscribe")
        self.assertIsNotNone(peer_event, "Did not receive on_peer_subscribe event")
        print(f"Publisher Event: {peer_event}")

        # 2. As the Subscriber, we should start receiving data (stats)
        # The mock blaster sends data every 100ms
        print("Waiting for data flow ('on_stat_update')...")
        stat_event = self.client.wait_for_event("on_stat_update")
        self.assertIsNotNone(stat_event, "Did not receive any data (on_stat_update)")
        print(f"Data received: {stat_event}")

        # Unsubscribe
        self.client.unsubscribe(sub_id)

if __name__ == '__main__':
    unittest.main()