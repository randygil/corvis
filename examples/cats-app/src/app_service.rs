use corvis::prelude::*;

#[injectable]
pub struct AppService;

impl AppService {
    pub fn get_hello(&self) -> String {
        "Hello World!".to_string()
    }
}
