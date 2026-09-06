# F3 部署三对账证据（2026-09-06 15:0x +0800）

## 运行中服务器进程 (PID 3949323, 监听 127.0.0.1:4317)
```
3949323 3949321 Sun Sep  6 15:00:18 2026 target/debug/aria web --workspace /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate
lrwxrwxrwx 1 michaelche michaelche 0 Sep  6 15:00 /proc/3949323/exe -> /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/target/debug/aria
71358a1a80a39814467817860659703b  /proc/3949323/exe
proc_exe inode=429521890
```

## 磁盘二进制（构建于 2026-09-06 15:00:11 +0800, cargo build --locked）
```
71358a1a80a39814467817860659703b  target/debug/aria
71358a1a80a39814467817860659703b  target/debug/aria
inode=246648769 mtime=2026-09-06 15:00:11.510539692 +0800 size=159794720
```

## 对账结论：md5(进程 exe)==md5(磁盘)=71358a1a80a39814467817860659703b（磁盘二次稳定）；/proc/PID/exe 符号链接指向工作树真实文件且非 deleted（注：对 /proc/PID/exe 做 stat 返回 procfs 伪 inode，不作对账依据；磁盘 inode=246648769 稳定）；health ok；启动日志 aria web listening on http://127.0.0.1:4317

## 过程异常记录（如实）：首次核对时 pgrep -f 自匹配误抓包装 bash（3949320, exe=/usr/bin/bash）导致一组错误 md5/inode 读数；复核后确认真实进程 3949323 三对账全过。
