//! Выражения `{{…}}` в строках сьюта (спека L2 §3.3). Разбор и подстановка — Task 2; здесь путь
//! ключа, который нужен `lines` уже сейчас.

/// Сегмент пути ключа в сьюте: `scenarios[0].steps[1].http.body`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}
