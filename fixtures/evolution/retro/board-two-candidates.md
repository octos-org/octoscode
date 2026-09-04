### EVO-0001（2026-09-05T01:00:00Z，harvest）
trigger: ack_blocked
source: review /repo/a/.octos/OUTER_LOOP_REVIEW.md
identity: board:/repo/a/.octos/OUTER_LOOP_REVIEW.md#12#blocked#1111111111111111111111111111111111111111111111111111111111111111
envelope: line=7 offset=118 ts=2026-09-05T01:00:00Z
symptom: ACK(blocked): cargo test fails with E0596 unresolved import
### EVO-0002（2026-09-05T01:00:01Z，harvest）
trigger: ack_blocked
source: review /repo/a/.octos/OUTER_LOOP_REVIEW.md
identity: board:/repo/a/.octos/OUTER_LOOP_REVIEW.md#13#blocked#2222222222222222222222222222222222222222222222222222222222222222
envelope: line=13 offset=273 ts=2026-09-05T01:00:01Z
symptom: ACK(blocked): cargo test fails with E0382 borrow error
### EVO-0003（2026-09-05T01:00:02Z，harvest）
trigger: turn_error
source: events /repo/a/events.jsonl
identity: events:/repo/a/events.jsonl#2026-09-05T01:00:02Z#turn_error#slug-x#3333333333333333333333333333333333333333333333333333333333333333
envelope: line=2 offset=103 ts=2026-09-05T01:00:02Z
symptom: {"ts":"2026-09-05T01:00:02Z","kind":"turn_error","slug":"slug-x","data":{"detail":"writer stalled"}}
