use crate::abi::*;
use crate::data::Data;
use std::collections::BTreeMap;
use std::ptr;

const MAX_ITEMS: usize = 65_536;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 64;

pub unsafe fn decode_arguments(
    arguments: *const Value,
    count: usize,
) -> Result<Vec<Data>, &'static str> {
    let values = unsafe { value_slice(arguments, count) }?;
    values
        .iter()
        .map(|value| unsafe { decode_value(value, 0) })
        .collect()
}

unsafe fn decode_value(value: &Value, depth: usize) -> Result<Data, &'static str> {
    if depth > MAX_DEPTH {
        return Err("SQLITE_VALUE_LIMIT");
    }
    Ok(match value.kind {
        NULL => Data::Nil,
        BOOL => Data::Bool(value.flags & FLAG_TRUE != 0),
        INTEGER => Data::Integer(unsafe { value.data.integer }),
        NUMBER => {
            let number = unsafe { value.data.number };
            if !number.is_finite() {
                return Err("SQLITE_VALUE_TYPE");
            }
            Data::Number(number)
        }
        STRING => Data::String(
            String::from_utf8(unsafe { copy_bytes(value) }?).map_err(|_| "SQLITE_VALUE_UTF8")?,
        ),
        BYTES => Data::Bytes(unsafe { copy_bytes(value) }?),
        ARRAY => {
            let count = usize::try_from(value.length).map_err(|_| "SQLITE_VALUE_LIMIT")?;
            let values = unsafe { value_slice(value.data.items, count) }?;
            Data::Array(
                values
                    .iter()
                    .map(|value| unsafe { decode_value(value, depth + 1) })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        }
        MAP => {
            let count = usize::try_from(value.length).map_err(|_| "SQLITE_VALUE_LIMIT")?;
            let values = unsafe {
                value_slice(
                    value.data.items,
                    count.checked_mul(2).ok_or("SQLITE_VALUE_LIMIT")?,
                )
            }?;
            let mut result = BTreeMap::new();
            for pair in values.chunks_exact(2) {
                let Data::String(key) = (unsafe { decode_value(&pair[0], depth + 1) })? else {
                    return Err("SQLITE_VALUE_TYPE");
                };
                let item = unsafe { decode_value(&pair[1], depth + 1) }?;
                if result.insert(key, item).is_some() {
                    return Err("SQLITE_VALUE_TYPE");
                }
            }
            Data::Map(result)
        }
        RESOURCE if value.flags & FLAG_RESOURCE_HANDLE != 0 => {
            Data::Resource(unsafe { value.data.handle })
        }
        _ => return Err("SQLITE_VALUE_TYPE"),
    })
}

unsafe fn copy_bytes(value: &Value) -> Result<Vec<u8>, &'static str> {
    let length = usize::try_from(value.length).map_err(|_| "SQLITE_VALUE_LIMIT")?;
    if length > MAX_BYTES {
        return Err("SQLITE_VALUE_LIMIT");
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    let pointer = unsafe { value.data.bytes };
    if pointer.is_null() {
        return Err("SQLITE_VALUE_TYPE");
    }
    Ok(unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec())
}

unsafe fn value_slice<'a>(
    pointer: *const Value,
    length: usize,
) -> Result<&'a [Value], &'static str> {
    if length > MAX_ITEMS {
        return Err("SQLITE_VALUE_LIMIT");
    }
    if length == 0 {
        return Ok(&[]);
    }
    if pointer.is_null() {
        return Err("SQLITE_VALUE_TYPE");
    }
    Ok(unsafe { std::slice::from_raw_parts(pointer, length) })
}

pub fn encode_data(data: Data) -> Value {
    match data {
        Data::Nil => Value::default(),
        Data::Bool(value) => Value {
            kind: BOOL,
            flags: if value { FLAG_TRUE } else { 0 },
            ..Value::default()
        },
        Data::Integer(value) => Value {
            kind: INTEGER,
            data: ValueData { integer: value },
            ..Value::default()
        },
        Data::Number(value) => Value {
            kind: NUMBER,
            data: ValueData { number: value },
            ..Value::default()
        },
        Data::String(value) => encode_bytes(STRING, value.into_bytes()),
        Data::Bytes(value) => encode_bytes(BYTES, value),
        Data::Array(values) => {
            encode_children(ARRAY, values.into_iter().map(encode_data).collect(), false)
        }
        Data::Map(values) => {
            let mut children = Vec::with_capacity(values.len().saturating_mul(2));
            for (key, value) in values {
                children.push(encode_data(Data::String(key)));
                children.push(encode_data(value));
            }
            encode_children(MAP, children, true)
        }
        Data::Resource(handle) => Value {
            kind: RESOURCE,
            flags: FLAG_RESOURCE_HANDLE,
            data: ValueData { handle },
            ..Value::default()
        },
    }
}

fn encode_bytes(kind: u32, bytes: Vec<u8>) -> Value {
    if bytes.is_empty() {
        return Value {
            kind,
            ..Value::default()
        };
    }
    let bytes = bytes.into_boxed_slice();
    let length = bytes.len() as u64;
    let pointer = Box::into_raw(bytes).cast::<u8>();
    Value {
        kind,
        length,
        data: ValueData { bytes: pointer },
        ..Value::default()
    }
}

fn encode_children(kind: u32, children: Vec<Value>, map: bool) -> Value {
    let logical_length = if map {
        children.len() / 2
    } else {
        children.len()
    };
    if children.is_empty() {
        return Value {
            kind,
            ..Value::default()
        };
    }
    let children = children.into_boxed_slice();
    let pointer = Box::into_raw(children).cast::<Value>();
    Value {
        kind,
        length: logical_length as u64,
        data: ValueData { items: pointer },
        ..Value::default()
    }
}

pub unsafe extern "C" fn free_value(value: *mut Value) {
    let Some(value) = (unsafe { value.as_mut() }) else {
        return;
    };
    unsafe { free_value_inner(value) };
    *value = Value::default();
}

unsafe fn free_value_inner(value: &mut Value) {
    match value.kind {
        STRING | BYTES => {
            let length = usize::try_from(value.length).unwrap_or(0);
            let pointer = unsafe { value.data.bytes as *mut u8 };
            if length > 0 && !pointer.is_null() {
                drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(pointer, length)) });
            }
        }
        ARRAY | MAP | ERROR_VALUE => {
            let logical = usize::try_from(value.length).unwrap_or(0);
            let length = if value.kind == MAP {
                logical.saturating_mul(2)
            } else {
                logical
            };
            let pointer = unsafe { value.data.items as *mut Value };
            if length > 0 && !pointer.is_null() {
                let mut values =
                    unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(pointer, length)) };
                for value in &mut values {
                    unsafe { free_value_inner(value) };
                }
            }
        }
        RESOURCE => {
            let pointer = unsafe { value.data.resource };
            if !pointer.is_null() {
                let descriptor = unsafe { Box::from_raw(pointer) };
                if !descriptor.resource.is_null()
                    && let Some(drop_resource) = descriptor.drop_resource
                {
                    unsafe { drop_resource(descriptor.resource) };
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_frees_nested_values() {
        let mut map = BTreeMap::new();
        map.insert("文字".into(), Data::String("值".into()));
        map.insert("字节".into(), Data::Bytes(vec![0, 255]));
        let mut encoded = encode_data(Data::Map(map));
        assert_eq!(encoded.kind, MAP);
        unsafe { free_value(&mut encoded) };
        assert_eq!(encoded.kind, NULL);
    }
}
