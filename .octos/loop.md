# 维护循环(外环协作版,protocol: olp/v0)

每次维护唤醒依次执行,全部完成才结束本轮:

1. 读 `docs/OUTER_LOOP_REVIEW.md` 的 `Active` 区(外环审查黑板)。
2. 若 `Active` 存在**未 ACK 的条目**:取编号最小的一条,按其内容执行到完成
   (代码改动跑 `cargo test --all-targets` + fmt + clippy 后原子 commit,
   只 add 自己改的文件),然后在该条目下补
   `ACK(done|wontdo|blocked): <说明>`。
3. 若 `Active` 无待办:运行 `git status`。工作区干净则直接结束;存在来源
   不明的改动或文件则原样保留、报告并停止。不得清理或提交其他 actor 的
   工作;只可处理本轮由自己创建且路径已知的临时产物。
4. 遵守 AGENTS.md 的全部纪律;审批类操作永远留给 operator。
