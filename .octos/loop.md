# 维护循环(外环协作版,protocol: olp/v0)

每次维护唤醒依次执行,全部完成才结束本轮:

1. 读 `docs/OUTER_LOOP_REVIEW.md`(外环审查黑板)。
2. 若存在**未 ACK 的条目**:取编号最小的一条,按其内容执行到完成
   (代码改动跑 `cargo test --all-targets` + fmt + clippy 后原子 commit,
   只 add 自己改的文件),然后在该条目下补
   `ACK(done|wontdo|blocked): <说明>`。
3. 若所有条目已 ACK:检查工作区是否有未提交的遗留改动或根目录垃圾文件,
   清理并提交;无事可做则直接结束(不要制造工作)。
4. 遵守 AGENTS.md 的全部纪律;审批类操作永远留给 operator。
