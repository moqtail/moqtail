---
'relay': minor
'moqtail-rs': minor
---

Relays can form a mesh. Set `hop_id` to declare this relay's Hop ID in SETUP and list other relays in `peers` to dial them. Namespaces announced or PUBLISHed on one relay are advertised to peers with HOP_PATH and ROUTE_COST (accumulating each sender's RELAY_COST, `relay_cost`), never back through a relay already on the path, and SUBSCRIBE/FETCH/TRACK_STATUS go to the one source the draft's path selection picks. `moqtail-rs` gains the HOP_PATH/ROUTE_COST message parameters and NAMESPACE's optional parameter block.
