#![forbid(unsafe_code)]
use serde::{Deserialize,Serialize};
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)] pub struct AppConfig { pub library_root:Option<String>,pub minimum_width:u32,pub minimum_height:u32,pub allow_unknown_dimensions:bool }
impl Default for AppConfig { fn default()->Self { Self{library_root:None,minimum_width:800,minimum_height:800,allow_unknown_dimensions:false} } }
