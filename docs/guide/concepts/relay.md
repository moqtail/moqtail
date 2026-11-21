---
description: How the MOQtail relay fans one upstream subscription out to many subscribers, caches groups, serves FETCH, copes with slow subscribers and routes across a relay mesh.
---

# How the relay forwards

The relay is a subscriber to every publisher it reaches and a publisher to every subscriber that reaches it. This page explains what happens in between. The settings named here are in the [configuration file](/relay/reference/config), and [Configure the relay](/relay/configure) shows how to set them.

## One upstream subscription, many subscribers

The relay keeps one track for each full track name. The first SUBSCRIBE for a track creates it, and the relay subscribes upstream: to every publisher of that exact track, and to every publisher that announced a namespace the track falls under. Later subscribers join the track that exists, without a new request upstream. However many viewers watch, each object crosses the link from the publisher once.

The upstream SUBSCRIBE is the relay's own, not a copy of the first subscriber's. It starts at the next object, in ascending group order, with nothing filtered. Each subscriber's own start, priority, group order and filters are applied on the way out, separately for each subscriber. So the first subscriber does not decide what the others can get.

When several publishers serve one track, the relay remembers which objects it already forwarded in recent groups (`dedup_retained_groups`) and sends each object on once.

A track lives as long as something feeds it. When a track that a SUBSCRIBE created loses its last subscriber, the relay cancels the upstream subscription. A track that a publisher pushed with PUBLISH stays for as long as the publisher sends it. Either way, a track that still holds cached objects stays as a cache-only track and keeps serving FETCH from its cache. The next SUBSCRIBE or PUBLISH for it takes it over.

## The cache

Every object the relay receives, from a subscription or an upstream FETCH, goes into its track's cache, kept by group. The cache holds up to `cache_size` groups per track. A group leaves it `cache_expiration` after it was cached (`cache_expiration_type: ttl`), or after it was last used (`tti`). A track that sets a MAX_CACHE_DURATION property can shorten that.

The cache also remembers ranges of locations known to hold no object. That an object does not exist never changes, so these ranges outlive the groups they came from.

## Serving FETCH

A FETCH asks for a range of past objects. The relay first settles where the range ends, which FETCH_OK has to state. When the request ends at or before the largest object the relay has seen, it answers from what it holds. Otherwise the publisher may know of later objects, so the relay forwards the FETCH upstream first and answers once the publisher has said where the range ends. It never answers early with a shorter range.

Then it delivers the range group by group. A group comes from the cache only when the cache accounts for every location the request wants from it, as an object or as a known absence. A run of groups the cache lacks is one gap, and the relay asks upstream for it with one FETCH, forwarding the objects as they arrive and caching them on the way. When two FETCHes need the same gap at once, the second waits for the first one's fill instead of asking again.

One FETCH sends at most `max_upstream_fetch_gaps` gaps upstream. After that, each further gap goes to the subscriber as an End of Unknown Range marker, which says the relay does not know what those locations hold, rather than that they are empty.

A FETCH for a track the relay has never seen creates a cache-only track and fills it the same way.

## Track aliases

A subscriber learns its track alias from the relay's SUBSCRIBE_OK, and every data stream the relay sends it carries that alias. MOQT lets a publisher give the subscriptions to one track the same alias or different ones, so `track_alias_mode` picks:

- `per-subscription`, the default, gives each subscription a new alias, unique within the subscriber's session.
- `per-track` gives every subscription to a track the same alias, the relay's own id for the track.

A track the relay pushes with PUBLISH uses the alias in that PUBLISH.

Streams and the messages that set aliases travel separately, so a stream can arrive before its alias is known. The relay holds a publisher's stream for up to `track_alias_resolution_timeout` while it waits for the PUBLISH or SUBSCRIBE_OK that sets the alias. On the way out, it queues a subscriber's objects for up to `downstream_alias_timeout` until its SUBSCRIBE_OK has been sent.

## Rendezvous

A SUBSCRIBE can arrive before any publisher of its track. By default the relay then answers at once with REQUEST_ERROR DOES_NOT_EXIST. A subscriber that sends RENDEZVOUS_TIMEOUT asks the relay to wait instead: the SUBSCRIBE is held until a publisher announces or publishes a match, or until the time runs out, and then fails with TIMEOUT. The relay caps the wait at `max_rendezvous_timeout`, so held requests can't tie up streams for long. With `0s` it never waits.

## Slow subscribers

The relay never slows a publisher down for one subscriber. Each subscription has its own queue, and the relay drops from that queue rather than wait:

- When an object would take a subscriber's queue past `max_subscriber_queue_bytes`, the relay skips the rest of that group for the subscriber. An empty queue always takes the next object, however large.
- When an object has waited in the queue longer than its delivery timeout, the relay skips the rest of its group too. The timeout is the subscriber's OBJECT_DELIVERY_TIMEOUT, capped by the relay's `delivery_timeout`. With neither set, which is the default, objects wait as long as they need.
- When a subscriber has more than `max_subscriber_lag` objects queued, the relay resets its data streams with TOO_FAR_BEHIND and ends the subscription. This check is off by default.

Skipping works in whole groups because a group starts where a decoder can start. A subscriber that loses the rest of one group picks up cleanly at the start of a later one.

When the connection itself is the bottleneck, QUIC decides which stream sends next. The relay gives each stream a priority from the subscriber priority, then the publisher priority, then the group's place in the subscription's group order (see [priorities](/guide/concepts/moqt#priorities-and-group-order)).

## The relay mesh

Relays can join a mesh, so a viewer on one relay can watch a publisher on another. Clustering is off unless the relay sets a `hop_id`. A relay dials the relays in its `peers` over raw QUIC and keeps the link up, redialing when it drops. One link carries announcements in both directions, so only one side has to list the other.

Over each link, a relay announces every namespace it can reach. Each announcement carries the hop IDs of the relays it passed through and the cost of reaching the source. A relay never offers a peer a route that already passed through that peer, so announcements can't loop. When it knows several routes to a namespace, it picks the cheapest, then the one with fewest hops. Each relay declares the cost of its own egress with `relay_cost`, and a route's cost adds up those of the relays on it. A relay that leaves `relay_cost` unset counts as 1, so a mesh without costs routes by hop count. A relay that is already serving a track under a namespace to a subscriber announces it at cost 0, since that traffic reaches it anyway.

SUBSCRIBE, FETCH and TRACK_STATUS follow the same choice to the source. Tracks that a client pushes with PUBLISH reach the peers whose own clients asked for them with SUBSCRIBE_TRACKS. [Build a relay mesh](/relay/clustering) shows how to set one up.

## Sender-side track switching

With sender-side track switching (SSTS), the relay chooses a subscriber's rendition instead of the player. The player subscribes to every track of a switching set at once and tells the relay they belong together. The relay then estimates that subscriber's bandwidth and, group by group, forwards one track of each set, using an allocation algorithm the two sides agreed on in SETUP. The draft is provisional, so the relay offers SSTS only when `ssts_enable` is set, and the player only with the `ssts()` plugin.

## Related

- [Configure the relay](/relay/configure)
- [Build a relay mesh](/relay/clustering)
- [Configuration file](/relay/reference/config)
- [MOQT in brief](/guide/concepts/moqt)
