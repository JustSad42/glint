//! Calls into the Rust side (`crate::handle`), run off the UI thread like the old
//! `invoke('call')`: file work and COM calls never block a frame.
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;

pub type Reply = Result<Value, String>;

pub struct Jobs<T> {
    tx: Sender<(T, Reply)>,
    rx: Receiver<(T, Reply)>,
}

impl<T: Send + 'static> Default for Jobs<T> {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Jobs { tx, rx }
    }
}

impl<T: Send + 'static> Jobs<T> {
    pub fn call(&self, ctx: &egui::Context, tag: T, cmd: &str, args: Value) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let cmd = cmd.to_string();
        std::thread::spawn(move || {
            let r = crate::handle(&cmd, args);
            let _ = tx.send((tag, r));
            ctx.request_repaint();
        });
    }

    pub fn poll(&self) -> Vec<(T, Reply)> {
        self.rx.try_iter().collect()
    }
}

/// `it["k"]` as a str ("" when missing).
pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

pub fn b(v: &Value, k: &str) -> bool {
    v[k].as_bool().unwrap_or(false)
}

/// JS truthiness for a field (non-empty string, true, non-zero number, object, array).
pub fn truthy(v: &Value, k: &str) -> bool {
    match &v[k] {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

pub fn n(v: &Value, k: &str) -> Option<f64> {
    v[k].as_f64()
}
