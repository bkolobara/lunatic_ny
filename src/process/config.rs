// Configuration structure for environments
#[derive(Debug)]
pub struct EnvConfig {
    // Maximum amount of memory that can be used by processes
    max_memory: u64,
    // Maximum amount of compute expressed in gallons.
    max_fuel: Option<u32>,
    allowed_namespaces: Vec<String>,
}

impl EnvConfig {
    pub fn new(max_memory: u64, max_fuel: Option<u32>) -> Self {
        Self {
            max_memory,
            max_fuel,
            allowed_namespaces: Vec::new(),
        }
    }

    pub fn max_memory(&self) -> u64 {
        self.max_memory
    }

    pub fn max_fuel(&self) -> Option<u32> {
        self.max_fuel
    }

    pub fn allowed_namespace(&self) -> &Vec<String> {
        &self.allowed_namespaces
    }

    pub fn allow_namespace<S: Into<String>>(&mut self, namespace: S) {
        self.allowed_namespaces.push(namespace.into())
    }
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            max_memory: 0x40000000, // 4 Gb
            max_fuel: None,
            allowed_namespaces: vec![
                String::from("lunatic::"),
                String::from("wasi_snapshot_preview1::"),
            ],
        }
    }
}