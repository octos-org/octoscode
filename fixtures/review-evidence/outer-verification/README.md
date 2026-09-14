# outer-verification (#629 residual 独立执行证据)

来源: 外层在独立 worktree ../verify-octoscode 实际执行的结果(非本仓库重放)。
base=0a174d95ddec pr_head=9bcf4099c271 base_exit=101 head_exit=101
分类: existing/residual: same original production event probe fails at BASE and PR HEAD

文件: outer-verification-slot.json / outer-629-base.log / outer-629-head.log / residual-629-repro.rs
这些是只读复制;residual 判定=同一生产 probe 在 BASE 与 HEAD 均失败,severity/引入维度独立。
