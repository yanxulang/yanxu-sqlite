mod abi;
mod backend;
mod bridge;
mod data;

use abi::*;
use backend::{HostApi, Operation, Output};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::OnceLock;

static MODULE: OnceLock<usize> = OnceLock::new();
static MODULE_NAME: &[u8] = "言舟".as_bytes();
static ERROR_MESSAGE: &[u8] = b"yanxu-sqlite rejected the native operation";
static PANIC_CODE: &[u8] = b"SQLITE_NATIVE_PANIC";
static PANIC_MESSAGE: &[u8] = b"panic isolated inside yanxu-sqlite native backend";

static FUNCTIONS: &[(&[u8], Operation)] = &[
    ("打开".as_bytes(), Operation::Open),
    ("执行".as_bytes(), Operation::Execute),
    ("查询".as_bytes(), Operation::Query),
    ("信息".as_bytes(), Operation::Information),
    ("关闭".as_bytes(), Operation::Close),
    ("开始事务".as_bytes(), Operation::Begin),
    ("提交".as_bytes(), Operation::Commit),
    ("回滚".as_bytes(), Operation::Rollback),
    ("保存点".as_bytes(), Operation::Savepoint),
    ("回滚至".as_bytes(), Operation::RollbackTo),
    ("释放点".as_bytes(), Operation::Release),
    ("准备语句".as_bytes(), Operation::Prepare),
    ("执行语句".as_bytes(), Operation::StatementExecute),
    ("查询语句".as_bytes(), Operation::StatementQuery),
    ("语句信息".as_bytes(), Operation::StatementInformation),
    ("表清单".as_bytes(), Operation::Tables),
    ("表结构".as_bytes(), Operation::TableStructure),
];

static RESOURCE_TYPES: &[&[u8]] = &[backend::CONNECTION_TYPE, backend::STATEMENT_TYPE];

#[unsafe(no_mangle)]
pub extern "C" fn yanxu_native_module_v2() -> *const NativeModule {
    *MODULE.get_or_init(|| {
        let functions = Box::leak(
            FUNCTIONS
                .iter()
                .map(|(name, operation)| NativeFunction {
                    name: name.as_ptr(),
                    name_length: name.len(),
                    context: (*operation as usize) as *mut c_void,
                    call: Some(dispatch),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let resource_types = Box::leak(
            RESOURCE_TYPES
                .iter()
                .map(|name| name.as_ptr())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let resource_lengths = Box::leak(
            RESOURCE_TYPES
                .iter()
                .map(|name| name.len())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        Box::into_raw(Box::new(NativeModule {
            abi_version: ABI,
            struct_size: std::mem::size_of::<NativeModule>(),
            name: MODULE_NAME.as_ptr(),
            name_length: MODULE_NAME.len(),
            functions: functions.as_ptr(),
            function_count: functions.len(),
            constants: ptr::null(),
            constant_count: 0,
            resource_types: resource_types.as_ptr(),
            resource_type_lengths: resource_lengths.as_ptr(),
            resource_type_count: resource_types.len(),
            free_value: Some(bridge::free_value),
            capabilities: 0b1_1111,
        })) as usize
    }) as *const NativeModule
}

unsafe extern "C" fn dispatch(
    context: *mut c_void,
    arguments: *const Value,
    count: usize,
    host: *const NativeHost,
    output: *mut Value,
    error: *mut NativeError,
) -> i32 {
    if output.is_null() || host.is_null() {
        return fail(error, "SQLITE_HOST_ABI");
    }
    let Some(operation) = Operation::from_context(context) else {
        return fail(error, "SQLITE_FUNCTION");
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let arguments = unsafe { bridge::decode_arguments(arguments, count) }?;
        let host = HostApi(unsafe { *host });
        unsafe { backend::call(operation, &arguments, host) }
    }));
    match result {
        Ok(Ok(Output::Value(value))) => {
            unsafe { *output = bridge::encode_data(value) };
            OK
        }
        Ok(Ok(Output::Resource(mut resource))) => {
            let raw = resource.take_resource();
            let descriptor = Box::new(NativeResource {
                struct_size: std::mem::size_of::<NativeResource>(),
                resource: raw,
                type_name: resource.type_name.as_ptr(),
                type_name_length: resource.type_name.len(),
                parent: resource.parent,
                drop_resource: Some(resource.drop_resource),
            });
            unsafe {
                *output = Value {
                    kind: RESOURCE,
                    data: ValueData {
                        resource: Box::into_raw(descriptor),
                    },
                    ..Value::default()
                }
            };
            OK
        }
        Ok(Err(code)) => fail(error, code),
        Err(_) => {
            if let Some(error) = unsafe { error.as_mut() } {
                *error = NativeError {
                    code: PANIC_CODE.as_ptr(),
                    code_length: PANIC_CODE.len(),
                    message: PANIC_MESSAGE.as_ptr(),
                    message_length: PANIC_MESSAGE.len(),
                };
            }
            ERROR
        }
    }
}

fn fail(error: *mut NativeError, code: &'static str) -> i32 {
    if let Some(error) = unsafe { error.as_mut() } {
        *error = NativeError {
            code: code.as_ptr(),
            code_length: code.len(),
            message: ERROR_MESSAGE.as_ptr(),
            message_length: ERROR_MESSAGE.len(),
        };
    }
    ERROR
}
