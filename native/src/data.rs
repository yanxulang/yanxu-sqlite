use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Nil,
    Bool(bool),
    Integer(i64),
    Number(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Data>),
    Map(BTreeMap<String, Data>),
    Resource(u64),
}

impl Data {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Data]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Data>> {
        match self {
            Self::Map(value) => Some(value),
            _ => None,
        }
    }
}
