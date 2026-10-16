---
'moqtail-ssts': minor
---

Stop the handoff overlap from costing the climb. A mid-group depth sample
that does not fire the monitor now touches nothing: the single deep reading
that is the normal stream handoff — the new group's stream open while the
previous one's FIN-ACK is still in flight — no longer zeroes the clean
streak the upshift counts on, and no longer ends the exploration. Only a
fired mid-group downshift costs a fresh streak. The boundary's reading
keeps its certainty: two streams of the delivered tier still open when a
group's first Object arrives is two groups behind, and one such reading
downshifts alone.
