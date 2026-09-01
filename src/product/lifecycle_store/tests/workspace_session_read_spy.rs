use std::cell::Cell;

thread_local! {
    static WORKSPACE_SESSION_READ_COUNT: Cell<usize> = const { Cell::new(0) };
    static WORKSPACE_SESSION_READ_PANIC: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn reset_workspace_session_read_spy() {
    WORKSPACE_SESSION_READ_COUNT.with(|count| count.set(0));
    WORKSPACE_SESSION_READ_PANIC.with(|panic| panic.set(false));
}

pub(crate) fn workspace_session_read_count() -> usize {
    WORKSPACE_SESSION_READ_COUNT.with(Cell::get)
}

pub(crate) fn set_workspace_session_read_panic(enabled: bool) {
    WORKSPACE_SESSION_READ_PANIC.with(|panic| panic.set(enabled));
}

pub(crate) fn record_workspace_session_read() {
    WORKSPACE_SESSION_READ_COUNT.with(|count| count.set(count.get() + 1));
    if WORKSPACE_SESSION_READ_PANIC.with(Cell::get) {
        panic!("unexpected LifecycleStore::get_workspace_session call");
    }
}
