use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InsertAnchor {
    Left,
    Right,
    Heading,
    None,
}

impl InsertAnchor {
    pub fn uses_template(&self) -> bool {
        matches!(self, InsertAnchor::Left | InsertAnchor::Right)
    }
}


#[derive(Copy, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaginateOptions {
    None,
    Num(usize),
    Date,
}

impl PaginateOptions {
    pub fn unwrap_num(self) -> usize {
        match self {
            PaginateOptions::None => {panic!("called `PaginateOptions::unwrap_num()` on a `None` value")}
            PaginateOptions::Num(num) => {num}
            PaginateOptions::Date => {panic!("called `PaginateOptions::unwrap_num()` on a `Date` value")}
        }
    }
}