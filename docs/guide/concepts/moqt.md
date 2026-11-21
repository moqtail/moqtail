---
description: The MOQT ideas MOQtail is built on, for readers who know video streaming but not MOQT.
---

# MOQT in brief

Media over QUIC Transport (MOQT) moves media as named tracks of small objects, through relays, over QUIC. This page covers the parts you meet when you use MOQtail, which follows [draft-ietf-moq-transport-22](https://datatracker.ietf.org/doc/draft-ietf-moq-transport/22/).

## Publishers, subscribers and relays

A publisher produces tracks, and a subscriber asks for them. A relay sits in between. It is a subscriber to the publisher and a publisher to its own subscribers, so one copy of a track coming in can serve many viewers going out. Relays can chain, so a request may cross several of them to reach its publisher.

Each endpoint opens one session to its peer, over WebTransport or raw QUIC (see [Transports](/guide/concepts/transports)). Both sides start with a SETUP message, which states the options and extensions they support. After that, every request opens a bidirectional stream of its own and gets its answer on that stream: an OK, or a REQUEST_ERROR with a code and a reason.

## Namespaces and tracks

A track is named by its full track name: a namespace and a track name. The namespace is a tuple, an ordered list of up to 32 fields, such as `moqtail` and `ch07`. The track name is one more field, such as `video` or `catalog`. Fields are bytes, not text, and a full track name is at most 4096 bytes long.

Because the namespace is a tuple, it works as a hierarchy. A publisher announces a namespace with PUBLISH_NAMESPACE, and a relay then sends it every request for a track under that namespace. A subscriber can also ask about everything under a prefix: SUBSCRIBE_NAMESPACE lists the namespaces below it, and SUBSCRIBE_TRACKS has the relay push each track below it with a PUBLISH message. A publisher can push a single track with PUBLISH without being asked.

## Groups, subgroups and objects

A track is a sequence of groups, and a group is a sequence of objects. An object's location is its group ID and object ID. It carries a payload, which MOQT never reads, and optional properties: key-value pairs such as a timestamp, which endpoints can read without parsing the payload.

This shape follows how video is decoded. A group starts where a decoder can start, usually a key frame, so it is where a subscriber joins and where a relay resumes a subscriber that fell behind. The objects after it usually depend on the ones before them, like the frames of a GOP.

A group is split into one or more subgroups, and each subgroup travels on a QUIC stream of its own. Objects on one stream arrive in order, and a lost packet delays only the stream it belongs to. A publisher that puts a whole group in one subgroup gets the GOP delivered in order. One that splits it, for example a base layer and an enhancement layer in separate subgroups, can give them different priorities, so the base layer goes first when the link is short.

Objects also mark endings: an object with the status End of Group or End of Track says that nothing more follows.

A data stream starts with a header that names its track by a track alias, a number, instead of the full track name. The publisher sets the alias when it accepts the subscription, in SUBSCRIBE_OK, or when it pushes the track, in PUBLISH. A relay may give the subscribers of one track the same alias or different ones (see [track aliases](/guide/concepts/relay#track-aliases)).

## Subscribe and fetch

SUBSCRIBE asks for a track's objects as they are published. Its LOCATION_FILTER says where to start: at the next object, at the start of the next group, or at a given location. It can also give an end. The subscription stays open until one side ends it. The publisher ends it with PUBLISH_DONE.

FETCH asks for objects that were already published: a range of locations, sent in order on one stream, which closes when the range is done. Its answer, FETCH_OK, says where the range really ends. Players use it for video on demand and for seeking.

The two meet in FILL_PARAMETERS. A SUBSCRIBE that carries it also gets the past objects of a range it names, on a fetch stream next to the live ones. The player uses it to start a live track behind the live edge, for example from a range in the URL: the subscription starts at the next object, and the fill covers everything from the chosen start up to it, with no gap and no overlap.

## Priorities and group order

When there is more to send than the link can carry, three values decide what goes first:

- **Subscriber priority**, from 0 to 255, set for each subscription or fetch with SUBSCRIBER_PRIORITY. Lower goes first. The default is 128.
- **Publisher priority**, from 0 to 255, set by the publisher for each subgroup or datagram. It orders what has the same subscriber priority.
- **Group order**, set for each subscription with GROUP_ORDER: ascending sends older groups first, descending sends the newest first, and original keeps the publisher's order.

The MOQtail relay maps them onto QUIC stream priorities in that order: subscriber priority, then publisher priority, then group. The player relies on this. It subscribes to the catalog at priority 0 and to media at 128, and the ABR controller's probing fetches run at 200, so they only use bandwidth that live media leaves idle.

A subscriber can also bound how long an object is worth waiting for, with OBJECT_DELIVERY_TIMEOUT. The MOQtail relay skips the rest of a group for a subscriber once one of its objects has waited longer.

## Datagrams

A publisher can send an object as a QUIC datagram instead of on a stream. The datagram holds the whole object, with its track alias and location. A lost datagram is not sent again, which suits objects that are worthless when late, such as audio in a call.

## Related

- [Transports](/guide/concepts/transports): WebTransport and raw QUIC
- [Catalogs and packaging](/guide/concepts/media): what the payloads hold
- [How the relay forwards](/guide/concepts/relay)
- [Subscribe and fetch](/ts/client/subscribe) with the TypeScript client
