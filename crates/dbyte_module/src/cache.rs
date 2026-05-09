#[derive(Debug, Clone)]
pub enum ModuleState<T> {
    Loading,
    Loaded(T),
}
