---
'moqtail': minor
---

A subgroup object without properties, sent on a stream whose header type has properties, now carries an empty properties field. It carried none, so a receiver read the rest of the stream wrong: a relay dropped every End of Group status object a publisher sent after an object with LOC properties. A published subgroup stream's header now always has properties, so a later object can carry them when the first had none.

Breaking: `SubgroupObject.serialize(previousObjectId, headerType)` requires the stream's header type, which decides whether the object carries a properties field. It throws a `ProtocolViolationError` for an object with properties under a header type without them, and `SendStream.write` throws a `TypeError` for a subgroup object on a fetch stream.
