---
'relay': minor
---

The mid-group sample clock is the delivery itself. The controller's fixed
100 ms interval is gone: every Object successfully written to a
subscriber's send stream sends `ObjectSent` to the controller, which takes
the mid-group reading there. Samples are as regular as the path delivers
(~33 ms median on the sample media, thinning as the path slows), and a
connection that sends nothing takes no mid-group samples — the accepted
cost is blindness during a true zero-progress stall, where the depth
reading freezes with the delivery instead of being re-read on a wall
clock. Measured on the cascade with wasd-v2: same decisions (25
downshifts), cheaper ones (waste 3.23 -> 1.47 MB, zero delivery-timeout
resets, zero late frames), because congestion is confirmed after two
deliveries instead of two hundred milliseconds.
