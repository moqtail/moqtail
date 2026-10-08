---
'moqtail-ssts': minor
'relay': patch
---

Split WASD into the three versions the comparison measures. One state
machine — the depth read at the delivered tier, the mid-group monitor, the
same streaks, the same `wasd.reads` knob — advertised under three ids that
differ only in how far a downshift reaches back into the groups it overtook:

- `wasd` (`0xff04`), the classic: retroactive nothing.
- `wasd-v2` (`0xff05`): the downshift cancels the newest overtaken group and
  has it re-sent at the new rung.
- `wasd-v3` (`0xff06`): plus one step back — every older overtaken group is
  cancelled too and re-sent one rung below the new one (the ladder's bottom
  being the floor).

Note the semantics change under an unchanged id: the staged reach that
`0xff04` had since the rename is now `0xff06`; `0xff04` became the classic.

The contract grows one field to make v2 expressible:
`Supersede::newest_only`, a request to reach only the newest of the overtaken
groups and leave the older ones to finish on the member they carry. The
relay honours it — no flip, no reset, no re-send mark for the older groups —
and `Selection::supersede_newest{,_with_deadline}` construct the request.
