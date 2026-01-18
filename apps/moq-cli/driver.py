# driver.py
import subprocess
import threading
import json
import time
import queue
import sys

class MoqClient:
    def __init__(self, binary_path="../../target/debug/moq-cli"):
        self.binary_path = binary_path
        self.proc = None
        self.msg_id = 0
        self.running = False
        
        # Queues for coordinating responses
        self.response_queues = {} # {req_id: Queue}
        self.events_queue = queue.Queue()
        
        # Store latest logs/events for assertion
        self.notifications = []

    def start(self):
        """Spawns the Rust binary."""
        try:
            self.proc = subprocess.Popen(
                [self.binary_path],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                bufsize=1 # Line buffered
            )
            self.running = True
            
            # Start background reader threads
            self.reader_thread = threading.Thread(target=self._read_loop, daemon=True)
            self.reader_thread.start()
            
            self.stderr_thread = threading.Thread(target=self._stderr_loop, daemon=True)
            self.stderr_thread.start()
            return True
        except FileNotFoundError:
            print(f"❌ Error: Could not find binary at {self.binary_path}")
            return False

    def stop(self):
        """Kills the binary."""
        self.running = False
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def _read_loop(self):
        while self.running and self.proc:
            line = self.proc.stdout.readline()
            if not line: break
            try:
                msg = json.loads(line)
                self._handle_message(msg)
            except json.JSONDecodeError:
                print(f"⚠️ Raw: {line.strip()}")

    def _stderr_loop(self):
        while self.running and self.proc:
            line = self.proc.stderr.readline()
            if not line: break
            # Print stderr directly to console for debugging
            sys.stderr.write(f"[RUST] {line}")

    def _handle_message(self, msg):
        # 1. Response to a command
        if "id" in msg and "result" in msg:
            req_id = msg["id"]
            if req_id in self.response_queues:
                self.response_queues[req_id].put(msg)
        
        # 2. Notification / Event
        elif "method" in msg:
            self.notifications.append(msg)
            self.events_queue.put(msg)

    def send_command(self, method, params=None):
        self.msg_id += 1
        req_id = self.msg_id
        req = {"jsonrpc": "2.0", "method": method, "params": params or {}, "id": req_id}
        
        q = queue.Queue()
        self.response_queues[req_id] = q
        
        if self.proc and self.proc.stdin:
            self.proc.stdin.write(json.dumps(req) + "\n")
            self.proc.stdin.flush()
        
        try:
            resp = q.get(timeout=2)
            return resp.get("result")
        except queue.Empty:
            return {"error": "Timeout"}

    def wait_for_event(self, method_name, timeout=5):
        """
        Waits for a specific notification method.
        Consumes from the queue until the event is found or timeout occurs.
        """
        start = time.time()
        
        while time.time() - start < timeout:
            # 1. Check history (in case we missed it while processing something else)
            # We iterate a copy to be safe
            for n in list(self.notifications):
                if n.get("method") == method_name:
                    return n
            
            # 2. Wait for new events
            try:
                remaining = timeout - (time.time() - start)
                if remaining <= 0: break
                
                # Block here until SOMETHING arrives
                msg = self.events_queue.get(timeout=remaining)
                
                # Check if it's what we want
                if msg.get("method") == method_name:
                    return msg
                
                # If not, loop again (we just consumed a log or other event)
                # Optional: print(f"   [Debug] Skipping {msg.get('method')}")
                
            except queue.Empty:
                continue
                
        return None

    # --- API Wrappers (Updated for Draft-16) ---

    def connect(self, url):
        return self.send_command("connect", {"url": url})

    def publish_namespace(self, namespace):
        # Renamed from announce
        return self.send_command("publish_namespace", {"namespace": namespace})

    def subscribe(self, namespace, track, **kwargs):
        """
        Subscribes to a track.
        Accepts optional Draft-16 params: 
        priority, group_order, filter_type, start_group, start_object, end_group, authorization_token
        """
        params = {"namespace": namespace, "track": track}
        # Merge any extra arguments into the params dictionary
        params.update(kwargs)
        return self.send_command("subscribe", params)
        
    def unsubscribe(self, sub_id):
        return self.send_command("unsubscribe", {"subscription_id": sub_id})

    def fetch(self, namespace, track, start_group, end_group, **kwargs):
        params = {
            "namespace": namespace,
            "track": track,
            "start_group": start_group,
            "start_object": 0,
            "end_group": end_group,
            "end_object": 0
        }
        params.update(kwargs)
        return self.send_command("fetch", params)