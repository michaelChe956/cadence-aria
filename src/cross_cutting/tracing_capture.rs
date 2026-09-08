//! 跨模块共享的 tracing 捕获 fixture（D① 诊断打点测试用）。
//!
//! 以**进程级全局 subscriber** 捕获 WARN 级日志到共享缓冲。必须用全局而非
//! `set_default` 线程局部挂载：tracing 静态 callsite 的 Interest 是进程级
//! 缓存——并行测试下，同一 callsite 若先在无 subscriber 的线程被评估（其他
//! 测试触发了同一位点），Interest 缓存为 never 后，线程局部 guard 无法挽回
//! （事件被短路丢弃，实测全量并行必现、单跑通过）。全局 subscriber 常驻使
//! Interest 恒定，捕获与线程归属解耦；断言使用特异字段值，跨测试串扰无影响。

use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::fmt::MakeWriter;

/// 进程级共享捕获缓冲（全局 subscriber 写入端）。
#[derive(Clone, Default)]
pub(crate) struct SharedTracingCapture {
    buffer: Arc<Mutex<String>>,
}

impl SharedTracingCapture {
    /// 取进程级捕获实例；首次调用时挂载全局捕获 subscriber（幂等，仅一次）。
    pub(crate) fn global() -> Self {
        static GLOBAL: OnceLock<SharedTracingCapture> = OnceLock::new();
        GLOBAL
            .get_or_init(|| {
                let capture = SharedTracingCapture::default();
                let subscriber = tracing_subscriber::fmt()
                    .with_max_level(tracing::Level::WARN)
                    .with_ansi(false)
                    .with_target(false)
                    .with_writer(capture.clone())
                    .finish();
                // 全局仅装一次：其他测试二进制内无竞争者；失败即已被本 fixture 装过。
                let _ = tracing::subscriber::set_global_default(subscriber);
                capture
            })
            .clone()
    }

    /// 当前捕获到的全部日志文本（含此前其他测试的输出；断言须用特异字段值）。
    pub(crate) fn captured(&self) -> String {
        self.buffer.lock().expect("tracing capture lock").clone()
    }
}

impl std::io::Write for SharedTracingCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer
            .lock()
            .expect("tracing capture lock")
            .push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedTracingCapture {
    type Writer = SharedTracingCapture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
