use std::ffi::c_void;

pub const ABI: u32 = 2;
pub const OK: i32 = 0;
pub const ERROR: i32 = 1;
pub const NULL: u32 = 0;
pub const BOOL: u32 = 1;
pub const INTEGER: u32 = 2;
pub const NUMBER: u32 = 3;
pub const STRING: u32 = 4;
pub const BYTES: u32 = 5;
pub const ARRAY: u32 = 6;
pub const MAP: u32 = 7;
pub const RESOURCE: u32 = 8;
pub const ERROR_VALUE: u32 = 10;
pub const FLAG_TRUE: u32 = 1;
pub const FLAG_RESOURCE_HANDLE: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub union ValueData {
    pub integer: i64,
    pub number: f64,
    pub bytes: *const u8,
    pub items: *const Value,
    pub resource: *mut NativeResource,
    pub handle: u64,
}

impl Default for ValueData {
    fn default() -> Self {
        Self { handle: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Value {
    pub kind: u32,
    pub flags: u32,
    pub length: u64,
    pub data: ValueData,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NativeError {
    pub code: *const u8,
    pub code_length: usize,
    pub message: *const u8,
    pub message_length: usize,
}

pub type DropResource = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
pub struct NativeResource {
    pub struct_size: usize,
    pub resource: *mut c_void,
    pub type_name: *const u8,
    pub type_name_length: usize,
    pub parent: u64,
    pub drop_resource: Option<DropResource>,
}

pub type CallbackRetain = unsafe extern "C" fn(*mut c_void, u64) -> i32;
pub type CallbackRelease = unsafe extern "C" fn(*mut c_void, u64) -> i32;
pub type CallbackPost =
    unsafe extern "C" fn(*mut c_void, u64, *const Value, usize, *mut NativeError) -> i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeHost {
    pub abi_version: u32,
    pub struct_size: usize,
    pub context: *mut c_void,
    pub callback_retain: Option<CallbackRetain>,
    pub callback_release: Option<CallbackRelease>,
    pub callback_post: Option<CallbackPost>,
    pub wake: Option<unsafe extern "C" fn(*mut c_void)>,
    pub pump: Option<unsafe extern "C" fn(*mut c_void, usize, *mut NativeError) -> i32>,
    pub has_permission: Option<unsafe extern "C" fn(*mut c_void, *const u8, usize) -> i32>,
    pub resource_get: Option<unsafe extern "C" fn(*mut c_void, u64, *mut *mut c_void) -> i32>,
    pub event_loop_id: u64,
    pub owner_thread_token: u64,
}

pub type NativeCall = unsafe extern "C" fn(
    *mut c_void,
    *const Value,
    usize,
    *const NativeHost,
    *mut Value,
    *mut NativeError,
) -> i32;

#[repr(C)]
pub struct NativeFunction {
    pub name: *const u8,
    pub name_length: usize,
    pub context: *mut c_void,
    pub call: Option<NativeCall>,
}

#[repr(C)]
pub struct NativeConstant {
    pub name: *const u8,
    pub name_length: usize,
    pub value: *const Value,
}

#[repr(C)]
pub struct NativeModule {
    pub abi_version: u32,
    pub struct_size: usize,
    pub name: *const u8,
    pub name_length: usize,
    pub functions: *const NativeFunction,
    pub function_count: usize,
    pub constants: *const NativeConstant,
    pub constant_count: usize,
    pub resource_types: *const *const u8,
    pub resource_type_lengths: *const usize,
    pub resource_type_count: usize,
    pub free_value: Option<unsafe extern "C" fn(*mut Value)>,
    pub capabilities: u64,
}
