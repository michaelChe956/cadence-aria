# 3.6 矩阵部署三对账（2026-09-07 13:23-26）

- PID 839954（--work-item-plan-single-candidate 同参）；exe 符号链接真实非 deleted
- md5 进程=磁盘=0a8ca4a30ee7d42fe55f3c1aa4124151 稳定×2（构建于 11:45，含九包全部 commits 至 cb797a0e）；health ok
- 观察记录：启动后瞬时幻读 25c72d83 一次（与 09-06 F3 部署同款），秒级自愈，inode/mtime/size 全程未变

# 3.6 终盘部署三对账（2026-09-10/11 新会话）

## v14（2026-09-10 23:2x，机器重启 /tmp 清空+二进制丢失后重建）
- 源码等价核验：HEAD b0a3868f 的 src 链顶=a9e822dd（v13 kind 打点本体），main 合并（c069157f）未触 src → Rust 源与 v13 等价
- 全新 release 构建 5m13s；PID 15538；md5 进程=磁盘=0cf3aad0e7b638818a76316b057611d7；health ok；同参（--work-item-plan-single-candidate）
- hub 托管（名 aria-36-v14），日志可随取（kind 打点捕获 rep1d 顶杀链）

## v15（2026-09-11 00:5x，误燃根修 23cdeb55 后）
- 构建 3m43s；PID 183996；md5 进程=磁盘=92c6ee9b9c386292e7fc72d9612cc986；health ok；唯一 aria 进程；同参
- v14 stop exit=1 属 stop 信号正常；hub 托管（名 aria-36-v15）
