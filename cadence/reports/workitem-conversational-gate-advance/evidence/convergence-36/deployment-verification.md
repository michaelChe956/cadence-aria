# 3.6 矩阵部署三对账（2026-09-07 13:23-26）

- PID 839954（--work-item-plan-single-candidate 同参）；exe 符号链接真实非 deleted
- md5 进程=磁盘=0a8ca4a30ee7d42fe55f3c1aa4124151 稳定×2（构建于 11:45，含九包全部 commits 至 cb797a0e）；health ok
- 观察记录：启动后瞬时幻读 25c72d83 一次（与 09-06 F3 部署同款），秒级自愈，inode/mtime/size 全程未变
