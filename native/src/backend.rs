use crate::abi::{self, DropResource, NativeHost};
use crate::data::Data;
use rusqlite::limits::Limit;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{
    Connection, Error as SqlError, ErrorCode, OpenFlags, OptionalExtension, params_from_iter,
};
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::time::Duration;

const CONNECTION_MAGIC: u64 = 0x5941_4e58_5553_514c;
const STATEMENT_MAGIC: u64 = 0x5941_4e58_5553_5454;
const MAX_SQL_BYTES: usize = 1024 * 1024;
const MAX_PARAMETERS: usize = 65_536;
const MAX_ROWS: usize = 100_000;
const MAX_COLUMNS: usize = 1024;
const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub static CONNECTION_TYPE: &[u8] = b"yanxu.sqlite.connection";
pub static STATEMENT_TYPE: &[u8] = b"yanxu.sqlite.statement";

#[derive(Clone, Copy)]
pub struct HostApi(pub NativeHost);

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Operation {
    Open = 1,
    Execute = 2,
    Query = 3,
    Information = 4,
    Close = 5,
    Begin = 6,
    Commit = 7,
    Rollback = 8,
    Savepoint = 9,
    RollbackTo = 10,
    Release = 11,
    Prepare = 12,
    StatementExecute = 13,
    StatementQuery = 14,
    StatementInformation = 15,
    Tables = 16,
    TableStructure = 17,
}

impl Operation {
    pub fn from_context(context: *mut c_void) -> Option<Self> {
        match context as usize {
            1 => Some(Self::Open),
            2 => Some(Self::Execute),
            3 => Some(Self::Query),
            4 => Some(Self::Information),
            5 => Some(Self::Close),
            6 => Some(Self::Begin),
            7 => Some(Self::Commit),
            8 => Some(Self::Rollback),
            9 => Some(Self::Savepoint),
            10 => Some(Self::RollbackTo),
            11 => Some(Self::Release),
            12 => Some(Self::Prepare),
            13 => Some(Self::StatementExecute),
            14 => Some(Self::StatementQuery),
            15 => Some(Self::StatementInformation),
            16 => Some(Self::Tables),
            17 => Some(Self::TableStructure),
            _ => None,
        }
    }
}

pub struct ResourceOutput {
    resource: *mut c_void,
    pub type_name: &'static [u8],
    pub parent: u64,
    pub drop_resource: DropResource,
}

impl ResourceOutput {
    fn new<T>(
        resource: Box<T>,
        type_name: &'static [u8],
        parent: u64,
        drop_resource: DropResource,
    ) -> Self {
        Self {
            resource: Box::into_raw(resource).cast::<c_void>(),
            type_name,
            parent,
            drop_resource,
        }
    }

    pub fn take_resource(&mut self) -> *mut c_void {
        std::mem::replace(&mut self.resource, std::ptr::null_mut())
    }
}

impl Drop for ResourceOutput {
    fn drop(&mut self) {
        if !self.resource.is_null() {
            unsafe { (self.drop_resource)(self.resource) };
        }
    }
}

pub enum Output {
    Value(Data),
    Resource(ResourceOutput),
}

pub struct ConnectionResource {
    magic: u64,
    connection: Option<Connection>,
}

pub struct StatementResource {
    magic: u64,
    sql: String,
    parameter_count: usize,
    parent_connection: u64,
}

struct QueryOutput {
    columns: Vec<String>,
    rows: Vec<Data>,
    metadata: Vec<Data>,
}

pub unsafe fn call(
    operation: Operation,
    arguments: &[Data],
    host: HostApi,
) -> Result<Output, &'static str> {
    match operation {
        Operation::Open => open(arguments).map(Output::Resource),
        Operation::Execute => {
            require_count(arguments, 3)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            let sql = text(&arguments[1])?;
            let parameters = parameters(&arguments[2])?;
            Ok(Output::Value(execute_sql(connection, sql, parameters)))
        }
        Operation::Query => {
            require_count(arguments, 3)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            let sql = text(&arguments[1])?;
            let parameters = parameters(&arguments[2])?;
            Ok(Output::Value(query_sql(connection, sql, parameters)))
        }
        Operation::Information => {
            require_count(arguments, 1)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            Ok(Output::Value(information(connection)))
        }
        Operation::Close => {
            require_count(arguments, 1)?;
            let (_, resource) = unsafe { connection_resource(arguments, host) }?;
            resource.connection.take();
            Ok(Output::Value(success_response(
                0,
                Data::Nil,
                BTreeMap::new(),
            )))
        }
        Operation::Begin => {
            require_count(arguments, 2)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            let mode = text(&arguments[1])?;
            Ok(Output::Value(begin_transaction(connection, mode)))
        }
        Operation::Commit => {
            require_count(arguments, 1)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            Ok(Output::Value(finish_transaction(connection, "COMMIT")))
        }
        Operation::Rollback => {
            require_count(arguments, 1)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            Ok(Output::Value(finish_transaction(connection, "ROLLBACK")))
        }
        Operation::Savepoint | Operation::RollbackTo | Operation::Release => {
            require_count(arguments, 2)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            let name = text(&arguments[1])?;
            Ok(Output::Value(savepoint_control(
                connection, operation, name,
            )))
        }
        Operation::Prepare => {
            require_count(arguments, 2)?;
            let (connection_handle, connection) = unsafe { connection(arguments, host) }?;
            let sql = text(&arguments[1])?;
            let statement = compile_statement(connection, connection_handle, sql)?;
            Ok(Output::Resource(ResourceOutput::new(
                Box::new(statement),
                STATEMENT_TYPE,
                connection_handle,
                drop_statement,
            )))
        }
        Operation::StatementExecute => {
            require_count(arguments, 2)?;
            let (statement, connection) = unsafe { prepared_context(arguments, host) }?;
            let parameters = parameters(&arguments[1])?;
            Ok(Output::Value(execute_prepared(
                connection, statement, parameters,
            )))
        }
        Operation::StatementQuery => {
            require_count(arguments, 2)?;
            let (statement, connection) = unsafe { prepared_context(arguments, host) }?;
            let parameters = parameters(&arguments[1])?;
            Ok(Output::Value(query_prepared(
                connection, statement, parameters,
            )))
        }
        Operation::StatementInformation => {
            require_count(arguments, 1)?;
            let (_, statement) = unsafe { statement_resource(arguments, host) }?;
            Ok(Output::Value(statement_information(statement)))
        }
        Operation::Tables => {
            require_count(arguments, 1)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            Ok(Output::Value(table_list(connection)?))
        }
        Operation::TableStructure => {
            require_count(arguments, 2)?;
            let (_, connection) = unsafe { connection(arguments, host) }?;
            let table = text(&arguments[1])?;
            Ok(Output::Value(table_structure(connection, table)?))
        }
    }
}

fn open(arguments: &[Data]) -> Result<ResourceOutput, &'static str> {
    require_count(arguments, 1)?;
    let config = map(&arguments[0])?;
    let path = config
        .get("路径")
        .and_then(Data::as_text)
        .ok_or("SQLITE_OPEN_PATH")?;
    if path.is_empty() || path.len() > 4096 || path.as_bytes().contains(&0) {
        return Err("SQLITE_OPEN_PATH");
    }
    let read_only = optional_bool(config, "只读", false)?;
    let create = optional_bool(config, "创建", !read_only)?;
    let uri = optional_bool(config, "URI", path.starts_with("file:"))?;
    let foreign_keys = optional_bool(config, "外键", true)?;
    let busy_timeout = optional_integer(config, "忙碌超时毫秒", 5000, 1, 604_800_000)?;
    let journal_mode = optional_text(
        config,
        "日志模式",
        if path == ":memory:" { "MEMORY" } else { "WAL" },
    )?;
    let synchronous = optional_text(config, "同步模式", "NORMAL")?;
    if !matches!(
        journal_mode,
        "DELETE" | "TRUNCATE" | "PERSIST" | "MEMORY" | "WAL" | "OFF"
    ) {
        return Err("SQLITE_OPEN_JOURNAL");
    }
    if !matches!(synchronous, "OFF" | "NORMAL" | "FULL" | "EXTRA") {
        return Err("SQLITE_OPEN_SYNCHRONOUS");
    }

    let mut flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if create && !read_only {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    if uri {
        flags |= OpenFlags::SQLITE_OPEN_URI;
    }
    let connection = Connection::open_with_flags(path, flags).map_err(|_| "SQLITE_OPEN")?;
    connection
        .busy_timeout(Duration::from_millis(busy_timeout as u64))
        .map_err(|_| "SQLITE_OPEN_TIMEOUT")?;
    connection
        .pragma_update(None, "foreign_keys", foreign_keys)
        .map_err(|_| "SQLITE_OPEN_FOREIGN_KEYS")?;
    if !read_only {
        connection
            .pragma_update(None, "journal_mode", journal_mode)
            .map_err(|_| "SQLITE_OPEN_JOURNAL")?;
        connection
            .pragma_update(None, "synchronous", synchronous)
            .map_err(|_| "SQLITE_OPEN_SYNCHRONOUS")?;
    }
    Ok(ResourceOutput::new(
        Box::new(ConnectionResource {
            magic: CONNECTION_MAGIC,
            connection: Some(connection),
        }),
        CONNECTION_TYPE,
        0,
        drop_connection,
    ))
}

fn compile_statement(
    connection: &mut Connection,
    parent_connection: u64,
    sql: &str,
) -> Result<StatementResource, &'static str> {
    validate_sql(sql)?;
    let statement = connection
        .prepare_cached(sql)
        .map_err(|_| "SQLITE_PREPARE")?;
    let parameter_count = statement.parameter_count();
    drop(statement);
    Ok(StatementResource {
        magic: STATEMENT_MAGIC,
        sql: sql.to_owned(),
        parameter_count,
        parent_connection,
    })
}

fn execute_sql(connection: &mut Connection, sql: &str, parameters: &[Data]) -> Data {
    if let Err(code) = validate_sql_and_parameters(sql, parameters) {
        return failure_response(code, "SQLite 执行参数无效");
    }
    let parameters = match sql_parameters(parameters) {
        Ok(parameters) => parameters,
        Err(code) => return failure_response(code, "SQLite 参数类型不受支持"),
    };
    let result = connection
        .prepare_cached(sql)
        .and_then(|mut statement| statement.execute(params_from_iter(parameters.iter())));
    match result {
        Ok(changed) => success_response(
            changed,
            safe_integer(connection.last_insert_rowid()),
            native_metadata(connection, Vec::new()),
        ),
        Err(error) => sqlite_failure(error),
    }
}

fn query_sql(connection: &mut Connection, sql: &str, parameters: &[Data]) -> Data {
    if let Err(code) = validate_sql_and_parameters(sql, parameters) {
        return failure_response(code, "SQLite 查询参数无效");
    }
    let parameters = match sql_parameters(parameters) {
        Ok(parameters) => parameters,
        Err(code) => return failure_response(code, "SQLite 参数类型不受支持"),
    };
    match query_rows(connection, sql, &parameters) {
        Ok(output) => {
            let mut response = BTreeMap::new();
            response.insert("成功".into(), Data::Bool(true));
            response.insert(
                "列名".into(),
                Data::Array(output.columns.into_iter().map(Data::String).collect()),
            );
            response.insert("行".into(), Data::Array(output.rows));
            response.insert("影响行数".into(), Data::Integer(0));
            response.insert(
                "元数据".into(),
                Data::Map(native_metadata(connection, output.metadata)),
            );
            Data::Map(response)
        }
        Err(error) => sqlite_failure(error),
    }
}

fn execute_prepared(
    connection: &mut Connection,
    statement: &StatementResource,
    parameters: &[Data],
) -> Data {
    let parameters = match prepared_parameters(statement, parameters) {
        Ok(parameters) => parameters,
        Err(code) => return failure_response(code, "SQLite 预编译语句参数无效"),
    };
    let result = connection
        .prepare_cached(&statement.sql)
        .and_then(|mut prepared| prepared.execute(params_from_iter(parameters.iter())));
    match result {
        Ok(changed) => success_response(
            changed,
            safe_integer(connection.last_insert_rowid()),
            native_metadata(connection, Vec::new()),
        ),
        Err(error) => sqlite_failure(error),
    }
}

fn query_prepared(
    connection: &mut Connection,
    statement: &StatementResource,
    parameters: &[Data],
) -> Data {
    let parameters = match prepared_parameters(statement, parameters) {
        Ok(parameters) => parameters,
        Err(code) => return failure_response(code, "SQLite 预编译语句参数无效"),
    };
    match query_rows(connection, &statement.sql, &parameters) {
        Ok(output) => {
            let mut response = BTreeMap::new();
            response.insert("成功".into(), Data::Bool(true));
            response.insert(
                "列名".into(),
                Data::Array(output.columns.into_iter().map(Data::String).collect()),
            );
            response.insert("行".into(), Data::Array(output.rows));
            response.insert("影响行数".into(), Data::Integer(0));
            response.insert(
                "元数据".into(),
                Data::Map(native_metadata(connection, output.metadata)),
            );
            Data::Map(response)
        }
        Err(error) => sqlite_failure(error),
    }
}

fn prepared_parameters(
    statement: &StatementResource,
    parameters: &[Data],
) -> Result<Vec<SqlValue>, &'static str> {
    if parameters.len() != statement.parameter_count {
        return Err("SQLITE_PARAMETER_COUNT");
    }
    sql_parameters(parameters)
}

fn statement_information(statement: &StatementResource) -> Data {
    let mut information = BTreeMap::new();
    information.insert("后端".into(), Data::String("native".into()));
    information.insert("SQL".into(), Data::String(statement.sql.clone()));
    information.insert(
        "参数数".into(),
        Data::Integer(statement.parameter_count as i64),
    );
    information.insert(
        "资源类型".into(),
        Data::String(String::from_utf8_lossy(STATEMENT_TYPE).into_owned()),
    );
    Data::Map(information)
}

fn table_list(connection: &mut Connection) -> Result<Data, &'static str> {
    let mut statement = connection
        .prepare_cached(
            "SELECT schema, name, type, ncol, wr, strict \
             FROM pragma_table_list \
             WHERE schema IN ('main', 'temp') AND name NOT GLOB 'sqlite_*' \
             ORDER BY schema, name",
        )
        .map_err(|_| "SQLITE_SCHEMA_REFLECTION")?;
    let rows = statement
        .query_map([], |row| {
            let mut item = BTreeMap::new();
            item.insert("模式".into(), Data::String(row.get(0)?));
            item.insert("名称".into(), Data::String(row.get(1)?));
            item.insert("种类".into(), Data::String(row.get(2)?));
            item.insert("列数".into(), Data::Integer(row.get(3)?));
            item.insert("无行号".into(), Data::Bool(row.get::<_, i64>(4)? != 0));
            item.insert("严格".into(), Data::Bool(row.get::<_, i64>(5)? != 0));
            Ok(Data::Map(item))
        })
        .map_err(|_| "SQLITE_SCHEMA_REFLECTION")?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|_| "SQLITE_SCHEMA_REFLECTION")?);
    }
    Ok(Data::Array(result))
}

fn table_structure(connection: &mut Connection, table: &str) -> Result<Data, &'static str> {
    validate_schema_name(table)?;
    let found = connection
        .query_row(
            "SELECT schema, type, ncol, wr, strict \
             FROM pragma_table_list \
             WHERE name = ?1 AND schema IN ('main', 'temp') \
             ORDER BY CASE schema WHEN 'temp' THEN 0 ELSE 1 END \
             LIMIT 1",
            [table],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "SQLITE_SCHEMA_REFLECTION")?;
    let Some((schema, kind, column_count, without_rowid, strict)) = found else {
        return Ok(missing_table_structure(table));
    };

    let catalog_sql = if schema == "temp" {
        "SELECT sql FROM sqlite_temp_schema WHERE name = ?1 AND type IN ('table', 'view')"
    } else {
        "SELECT sql FROM main.sqlite_schema WHERE name = ?1 AND type IN ('table', 'view')"
    };
    let create_sql = connection
        .query_row(catalog_sql, [table], |row| row.get::<_, Option<String>>(0))
        .optional()
        .map_err(|_| "SQLITE_SCHEMA_REFLECTION")?
        .flatten();
    let columns = column_information(connection, table).map_err(|_| "SQLITE_SCHEMA_REFLECTION")?;
    let indexes = index_information(connection, table).map_err(|_| "SQLITE_SCHEMA_REFLECTION")?;
    let foreign_keys =
        foreign_key_information(connection, table).map_err(|_| "SQLITE_SCHEMA_REFLECTION")?;

    let mut result = BTreeMap::new();
    result.insert("存在".into(), Data::Bool(true));
    result.insert("模式".into(), Data::String(schema));
    result.insert("名称".into(), Data::String(table.to_owned()));
    result.insert("种类".into(), Data::String(kind));
    result.insert("列数".into(), Data::Integer(column_count));
    result.insert("无行号".into(), Data::Bool(without_rowid != 0));
    result.insert("严格".into(), Data::Bool(strict != 0));
    result.insert("SQL".into(), create_sql.map_or(Data::Nil, Data::String));
    result.insert("列".into(), Data::Array(columns));
    result.insert("索引".into(), Data::Array(indexes));
    result.insert("外键".into(), Data::Array(foreign_keys));
    Ok(Data::Map(result))
}

fn validate_schema_name(table: &str) -> Result<(), &'static str> {
    if table.trim().is_empty() || table.len() > 512 || table.as_bytes().contains(&0) {
        return Err("SQLITE_SCHEMA_TABLE");
    }
    Ok(())
}

fn missing_table_structure(table: &str) -> Data {
    let mut result = BTreeMap::new();
    result.insert("存在".into(), Data::Bool(false));
    result.insert("模式".into(), Data::Nil);
    result.insert("名称".into(), Data::String(table.to_owned()));
    result.insert("种类".into(), Data::Nil);
    result.insert("列数".into(), Data::Integer(0));
    result.insert("无行号".into(), Data::Bool(false));
    result.insert("严格".into(), Data::Bool(false));
    result.insert("SQL".into(), Data::Nil);
    result.insert("列".into(), Data::Array(Vec::new()));
    result.insert("索引".into(), Data::Array(Vec::new()));
    result.insert("外键".into(), Data::Array(Vec::new()));
    Data::Map(result)
}

fn column_information(connection: &mut Connection, table: &str) -> Result<Vec<Data>, SqlError> {
    let mut statement = connection.prepare_cached(
        "SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden \
         FROM pragma_table_xinfo(?1) ORDER BY cid",
    )?;
    let rows = statement.query_map([table], |row| {
        let mut column = BTreeMap::new();
        column.insert("序号".into(), Data::Integer(row.get(0)?));
        column.insert("名称".into(), Data::String(row.get(1)?));
        column.insert("声明类型".into(), Data::String(row.get(2)?));
        column.insert("不可空".into(), Data::Bool(row.get::<_, i64>(3)? != 0));
        column.insert(
            "默认值".into(),
            row.get::<_, Option<String>>(4)?
                .map_or(Data::Nil, Data::String),
        );
        column.insert("主键序位".into(), Data::Integer(row.get(5)?));
        column.insert("隐藏".into(), Data::Integer(row.get(6)?));
        Ok(Data::Map(column))
    })?;
    rows.collect()
}

fn index_information(connection: &mut Connection, table: &str) -> Result<Vec<Data>, SqlError> {
    let indexes = {
        let mut statement = connection.prepare_cached(
            "SELECT seq, name, \"unique\", origin, partial \
             FROM pragma_index_list(?1) ORDER BY seq",
        )?;
        let rows = statement.query_map([table], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut result = Vec::with_capacity(indexes.len());
    for (sequence, name, unique, origin, partial) in indexes {
        let columns = index_columns(connection, &name)?;
        let mut index = BTreeMap::new();
        index.insert("序号".into(), Data::Integer(sequence));
        index.insert("名称".into(), Data::String(name));
        index.insert("唯一".into(), Data::Bool(unique != 0));
        index.insert("来源".into(), Data::String(origin));
        index.insert("部分".into(), Data::Bool(partial != 0));
        index.insert("列".into(), Data::Array(columns));
        result.push(Data::Map(index));
    }
    Ok(result)
}

fn index_columns(connection: &mut Connection, index: &str) -> Result<Vec<Data>, SqlError> {
    let mut statement = connection.prepare_cached(
        "SELECT seqno, cid, name, \"desc\", coll, \"key\" \
         FROM pragma_index_xinfo(?1) ORDER BY seqno",
    )?;
    let rows = statement.query_map([index], |row| {
        let mut column = BTreeMap::new();
        column.insert("序号".into(), Data::Integer(row.get(0)?));
        column.insert("表列序号".into(), Data::Integer(row.get(1)?));
        column.insert(
            "名称".into(),
            row.get::<_, Option<String>>(2)?
                .map_or(Data::Nil, Data::String),
        );
        column.insert("降序".into(), Data::Bool(row.get::<_, i64>(3)? != 0));
        column.insert(
            "排序规则".into(),
            row.get::<_, Option<String>>(4)?
                .map_or(Data::Nil, Data::String),
        );
        column.insert("键列".into(), Data::Bool(row.get::<_, i64>(5)? != 0));
        Ok(Data::Map(column))
    })?;
    rows.collect()
}

fn foreign_key_information(
    connection: &mut Connection,
    table: &str,
) -> Result<Vec<Data>, SqlError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, seq, \"table\", \"from\", \"to\", on_update, on_delete, \"match\" \
         FROM pragma_foreign_key_list(?1) ORDER BY id, seq",
    )?;
    let rows = statement.query_map([table], |row| {
        let mut foreign_key = BTreeMap::new();
        foreign_key.insert("编号".into(), Data::Integer(row.get(0)?));
        foreign_key.insert("序号".into(), Data::Integer(row.get(1)?));
        foreign_key.insert("目标表".into(), Data::String(row.get(2)?));
        foreign_key.insert("来源列".into(), Data::String(row.get(3)?));
        foreign_key.insert(
            "目标列".into(),
            row.get::<_, Option<String>>(4)?
                .map_or(Data::Nil, Data::String),
        );
        foreign_key.insert("更新动作".into(), Data::String(row.get(5)?));
        foreign_key.insert("删除动作".into(), Data::String(row.get(6)?));
        foreign_key.insert("匹配".into(), Data::String(row.get(7)?));
        Ok(Data::Map(foreign_key))
    })?;
    rows.collect()
}

fn begin_transaction(connection: &mut Connection, mode: &str) -> Data {
    if !connection.is_autocommit() {
        return failure_response("SQLITE_TRANSACTION_ACTIVE", "SQLite 连接已在事务中");
    }
    let sql = match mode {
        "DEFERRED" => "BEGIN DEFERRED",
        "IMMEDIATE" => "BEGIN IMMEDIATE",
        "EXCLUSIVE" => "BEGIN EXCLUSIVE",
        _ => {
            return failure_response(
                "SQLITE_TRANSACTION_MODE",
                "SQLite 事务模式仅支持 DEFERRED、IMMEDIATE 或 EXCLUSIVE",
            );
        }
    };
    execute_control(connection, sql)
}

fn finish_transaction(connection: &mut Connection, sql: &str) -> Data {
    if connection.is_autocommit() {
        return failure_response("SQLITE_TRANSACTION_STATE", "SQLite 连接没有活跃事务");
    }
    execute_control(connection, sql)
}

fn savepoint_control(connection: &mut Connection, operation: Operation, name: &str) -> Data {
    if connection.is_autocommit() {
        return failure_response("SQLITE_TRANSACTION_STATE", "SQLite 连接没有活跃事务");
    }
    let name = match quote_savepoint(name) {
        Ok(name) => name,
        Err(code) => return failure_response(code, "SQLite 保存点名称无效"),
    };
    let sql = match operation {
        Operation::Savepoint => format!("SAVEPOINT {name}"),
        Operation::RollbackTo => format!("ROLLBACK TO SAVEPOINT {name}"),
        Operation::Release => format!("RELEASE SAVEPOINT {name}"),
        _ => unreachable!(),
    };
    execute_control(connection, &sql)
}

fn quote_savepoint(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 255 || name.as_bytes().contains(&0) {
        return Err("SQLITE_SAVEPOINT_NAME");
    }
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

fn execute_control(connection: &mut Connection, sql: &str) -> Data {
    match connection.execute_batch(sql) {
        Ok(()) => success_response(0, Data::Nil, native_metadata(connection, Vec::new())),
        Err(error) => sqlite_failure(error),
    }
}

fn query_rows(
    connection: &mut Connection,
    sql: &str,
    parameters: &[SqlValue],
) -> Result<QueryOutput, SqlError> {
    let mut statement = connection.prepare_cached(sql)?;
    if statement.column_count() > MAX_COLUMNS {
        return Err(SqlError::InvalidColumnIndex(statement.column_count()));
    }
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let column_metadata = statement
        .columns()
        .into_iter()
        .enumerate()
        .map(|(index, column)| {
            let mut value = BTreeMap::new();
            value.insert("名称".into(), Data::String(column.name().to_owned()));
            value.insert("序号".into(), Data::Integer(index as i64));
            value.insert(
                "声明类型".into(),
                column
                    .decl_type()
                    .map_or(Data::Nil, |value| Data::String(value.to_owned())),
            );
            Data::Map(value)
        })
        .collect::<Vec<_>>();
    let mut rows = statement.query(params_from_iter(parameters.iter()))?;
    let mut output = Vec::new();
    let mut bytes = 0_usize;
    while let Some(row) = rows.next()? {
        if output.len() >= MAX_ROWS {
            return Err(SqlError::ExecuteReturnedResults);
        }
        let mut result = BTreeMap::new();
        for (index, name) in columns.iter().enumerate() {
            let value = sql_value(row.get_ref(index)?);
            bytes = bytes
                .saturating_add(data_size(&value))
                .saturating_add(name.len());
            if bytes > MAX_RESULT_BYTES {
                return Err(SqlError::ExecuteReturnedResults);
            }
            result.insert(name.clone(), value);
        }
        output.push(Data::Map(result));
    }
    Ok(QueryOutput {
        columns,
        rows: output,
        metadata: column_metadata,
    })
}

fn information(connection: &mut Connection) -> Data {
    let version = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))
        .unwrap_or_else(|_| rusqlite::version().to_owned());
    let json1 = connection
        .query_row("SELECT json_valid('null')", [], |row| row.get::<_, i64>(0))
        .map(|value| value == 1)
        .unwrap_or(false);
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .unwrap_or_else(|_| "unknown".into());
    let foreign_keys = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map(|value| value == 1)
        .unwrap_or(false);
    let mut result = native_metadata(connection, Vec::new());
    result.insert("JSON1".into(), Data::Bool(json1));
    result.insert("日志模式".into(), Data::String(journal_mode));
    result.insert("外键".into(), Data::Bool(foreign_keys));
    result.insert("SQLite版本".into(), Data::String(version));
    result.insert(
        "最大参数数".into(),
        Data::Integer(
            connection
                .limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)
                .unwrap_or(999) as i64,
        ),
    );
    Data::Map(result)
}

fn native_metadata(connection: &Connection, columns: Vec<Data>) -> BTreeMap<String, Data> {
    let mut metadata = BTreeMap::new();
    metadata.insert("后端".into(), Data::String("native".into()));
    metadata.insert(
        "SQLite版本".into(),
        Data::String(rusqlite::version().to_owned()),
    );
    metadata.insert("列".into(), Data::Array(columns));
    metadata.insert("自动提交".into(), Data::Bool(connection.is_autocommit()));
    metadata
}

fn validate_sql_and_parameters(sql: &str, parameters: &[Data]) -> Result<(), &'static str> {
    validate_sql(sql)?;
    if parameters.len() > MAX_PARAMETERS {
        return Err("SQLITE_PARAMETER_LIMIT");
    }
    Ok(())
}

fn validate_sql(sql: &str) -> Result<(), &'static str> {
    if sql.trim().is_empty() {
        return Err("SQLITE_SQL_EMPTY");
    }
    if sql.len() > MAX_SQL_BYTES {
        return Err("SQLITE_SQL_LIMIT");
    }
    Ok(())
}

fn sql_parameters(parameters: &[Data]) -> Result<Vec<SqlValue>, &'static str> {
    parameters.iter().map(sql_parameter).collect()
}

fn sql_parameter(value: &Data) -> Result<SqlValue, &'static str> {
    match value {
        Data::Nil => Ok(SqlValue::Null),
        Data::Bool(value) => Ok(SqlValue::Integer(i64::from(*value))),
        Data::Integer(value) => Ok(SqlValue::Integer(*value)),
        Data::Number(value) if value.is_finite() => Ok(SqlValue::Real(*value)),
        Data::String(value) => Ok(SqlValue::Text(value.clone())),
        Data::Bytes(value) => Ok(SqlValue::Blob(value.clone())),
        _ => Err("SQLITE_PARAMETER_TYPE"),
    }
}

fn sql_value(value: ValueRef<'_>) -> Data {
    match value {
        ValueRef::Null => Data::Nil,
        ValueRef::Integer(value) => safe_integer(value),
        ValueRef::Real(value) => Data::Number(value),
        ValueRef::Text(value) => Data::String(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => Data::Bytes(value.to_vec()),
    }
}

fn safe_integer(value: i64) -> Data {
    if value.unsigned_abs() <= MAX_SAFE_INTEGER as u64 {
        Data::Integer(value)
    } else {
        Data::String(value.to_string())
    }
}

fn success_response(
    changed: usize,
    last_insert_id: Data,
    metadata: BTreeMap<String, Data>,
) -> Data {
    let mut response = BTreeMap::new();
    response.insert("成功".into(), Data::Bool(true));
    response.insert("行".into(), Data::Array(Vec::new()));
    response.insert("影响行数".into(), Data::Integer(changed as i64));
    response.insert("末插入号".into(), last_insert_id);
    response.insert("元数据".into(), Data::Map(metadata));
    Data::Map(response)
}

fn failure_response(code: &str, message: &str) -> Data {
    let mut response = BTreeMap::new();
    response.insert("成功".into(), Data::Bool(false));
    response.insert("代码".into(), Data::String(code.to_owned()));
    response.insert("消息".into(), Data::String(message.to_owned()));
    response.insert("行".into(), Data::Array(Vec::new()));
    response.insert("影响行数".into(), Data::Integer(0));
    Data::Map(response)
}

fn sqlite_failure(error: SqlError) -> Data {
    match error {
        SqlError::SqliteFailure(failure, message) => {
            let code = sqlite_code(failure.code);
            let mut response =
                match failure_response(code, message.as_deref().unwrap_or("SQLite 数据库操作失败"))
                {
                    Data::Map(response) => response,
                    _ => unreachable!(),
                };
            response.insert(
                "扩展代码".into(),
                Data::Integer(failure.extended_code as i64),
            );
            Data::Map(response)
        }
        _ => failure_response("SQLITE_OPERATION", "SQLite 数据库操作失败"),
    }
}

fn sqlite_code(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::DatabaseBusy => "SQLITE_BUSY",
        ErrorCode::DatabaseLocked => "SQLITE_LOCKED",
        ErrorCode::ReadOnly => "SQLITE_READONLY",
        ErrorCode::OperationInterrupted => "SQLITE_INTERRUPT",
        ErrorCode::SystemIoFailure => "SQLITE_IOERR",
        ErrorCode::DatabaseCorrupt => "SQLITE_CORRUPT",
        ErrorCode::NotFound => "SQLITE_NOTFOUND",
        ErrorCode::DiskFull => "SQLITE_FULL",
        ErrorCode::CannotOpen => "SQLITE_CANTOPEN",
        ErrorCode::FileLockingProtocolFailed => "SQLITE_PROTOCOL",
        ErrorCode::SchemaChanged => "SQLITE_SCHEMA",
        ErrorCode::TooBig => "SQLITE_TOOBIG",
        ErrorCode::ConstraintViolation => "SQLITE_CONSTRAINT",
        ErrorCode::TypeMismatch => "SQLITE_MISMATCH",
        ErrorCode::ApiMisuse => "SQLITE_MISUSE",
        _ => "SQLITE_ERROR",
    }
}

fn parameters(value: &Data) -> Result<&[Data], &'static str> {
    value.as_array().ok_or("SQLITE_ARGUMENT_TYPE")
}

fn require_count(arguments: &[Data], expected: usize) -> Result<(), &'static str> {
    (arguments.len() == expected)
        .then_some(())
        .ok_or("SQLITE_ARGUMENT_COUNT")
}

fn text(value: &Data) -> Result<&str, &'static str> {
    value.as_text().ok_or("SQLITE_ARGUMENT_TYPE")
}

fn map(value: &Data) -> Result<&BTreeMap<String, Data>, &'static str> {
    value.as_map().ok_or("SQLITE_ARGUMENT_TYPE")
}

fn optional_bool(
    values: &BTreeMap<String, Data>,
    key: &str,
    default: bool,
) -> Result<bool, &'static str> {
    match values.get(key) {
        None => Ok(default),
        Some(value) => value.as_bool().ok_or("SQLITE_OPEN_CONFIG"),
    }
}

fn optional_integer(
    values: &BTreeMap<String, Data>,
    key: &str,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> Result<i64, &'static str> {
    let value = match values.get(key) {
        None => default,
        Some(value) => value.as_integer().ok_or("SQLITE_OPEN_CONFIG")?,
    };
    if !(minimum..=maximum).contains(&value) {
        return Err("SQLITE_OPEN_CONFIG");
    }
    Ok(value)
}

fn optional_text<'a>(
    values: &'a BTreeMap<String, Data>,
    key: &str,
    default: &'a str,
) -> Result<&'a str, &'static str> {
    match values.get(key) {
        None => Ok(default),
        Some(value) => value.as_text().ok_or("SQLITE_OPEN_CONFIG"),
    }
}

unsafe fn connection<'a>(
    arguments: &[Data],
    host: HostApi,
) -> Result<(u64, &'a mut Connection), &'static str> {
    let (handle, resource) = unsafe { connection_resource(arguments, host) }?;
    let connection = resource
        .connection
        .as_mut()
        .ok_or("SQLITE_CONNECTION_CLOSED")?;
    Ok((handle, connection))
}

unsafe fn connection_resource<'a>(
    arguments: &[Data],
    host: HostApi,
) -> Result<(u64, &'a mut ConnectionResource), &'static str> {
    let Data::Resource(handle) = arguments.first().ok_or("SQLITE_ARGUMENT_COUNT")? else {
        return Err("SQLITE_ARGUMENT_TYPE");
    };
    let resource = unsafe { connection_by_handle(*handle, host) }?;
    Ok((*handle, resource))
}

unsafe fn connection_by_handle<'a>(
    handle: u64,
    host: HostApi,
) -> Result<&'a mut ConnectionResource, &'static str> {
    let getter = host.0.resource_get.ok_or("SQLITE_HOST_RESOURCE")?;
    let mut raw = std::ptr::null_mut();
    if unsafe { getter(host.0.context, handle, &mut raw) } != abi::OK || raw.is_null() {
        return Err("SQLITE_CONNECTION_CLOSED");
    }
    let resource = unsafe { &mut *raw.cast::<ConnectionResource>() };
    if resource.magic != CONNECTION_MAGIC {
        return Err("SQLITE_RESOURCE_TYPE");
    }
    Ok(resource)
}

unsafe fn statement_resource<'a>(
    arguments: &[Data],
    host: HostApi,
) -> Result<(u64, &'a StatementResource), &'static str> {
    let Data::Resource(handle) = arguments.first().ok_or("SQLITE_ARGUMENT_COUNT")? else {
        return Err("SQLITE_ARGUMENT_TYPE");
    };
    let getter = host.0.resource_get.ok_or("SQLITE_HOST_RESOURCE")?;
    let mut raw = std::ptr::null_mut();
    if unsafe { getter(host.0.context, *handle, &mut raw) } != abi::OK || raw.is_null() {
        return Err("SQLITE_STATEMENT_CLOSED");
    }
    let resource = unsafe { &*raw.cast::<StatementResource>() };
    if resource.magic != STATEMENT_MAGIC {
        return Err("SQLITE_RESOURCE_TYPE");
    }
    Ok((*handle, resource))
}

unsafe fn prepared_context<'a>(
    arguments: &[Data],
    host: HostApi,
) -> Result<(&'a StatementResource, &'a mut Connection), &'static str> {
    let (_, statement) = unsafe { statement_resource(arguments, host) }?;
    let connection_resource = unsafe { connection_by_handle(statement.parent_connection, host) }?;
    let connection = connection_resource
        .connection
        .as_mut()
        .ok_or("SQLITE_CONNECTION_CLOSED")?;
    Ok((statement, connection))
}

fn data_size(value: &Data) -> usize {
    match value {
        Data::Nil | Data::Bool(_) | Data::Integer(_) | Data::Number(_) | Data::Resource(_) => 8,
        Data::String(value) => value.len(),
        Data::Bytes(value) => value.len(),
        Data::Array(values) => values.iter().map(data_size).sum(),
        Data::Map(values) => values
            .iter()
            .map(|(key, value)| key.len().saturating_add(data_size(value)))
            .sum(),
    }
}

pub unsafe extern "C" fn drop_connection(resource: *mut c_void) {
    if !resource.is_null() {
        let mut resource = unsafe { Box::from_raw(resource.cast::<ConnectionResource>()) };
        resource.magic = 0;
        resource.connection.take();
    }
}

pub unsafe extern "C" fn drop_statement(resource: *mut c_void) {
    if !resource.is_null() {
        let mut resource = unsafe { Box::from_raw(resource.cast::<StatementResource>()) };
        resource.magic = 0;
        resource.sql.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        connection
    }

    #[test]
    fn executes_with_real_parameter_binding() {
        let mut connection = memory_connection();
        assert!(matches!(
            execute_sql(
                &mut connection,
                "CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
                &[],
            ),
            Data::Map(_)
        ));
        let payload = "x'); DROP TABLE items; --";
        execute_sql(
            &mut connection,
            "INSERT INTO items(value) VALUES (?)",
            &[Data::String(payload.into())],
        );
        let result = query_sql(
            &mut connection,
            "SELECT value FROM items WHERE value = ?",
            &[Data::String(payload.into())],
        );
        let Data::Map(result) = result else {
            panic!("query response must be a map");
        };
        assert_eq!(result.get("成功"), Some(&Data::Bool(true)));
        let Data::Array(rows) = result.get("行").unwrap() else {
            panic!("rows must be an array");
        };
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn round_trips_blob_and_preserves_large_integer_as_text() {
        let mut connection = memory_connection();
        execute_sql(
            &mut connection,
            "CREATE TABLE values_table(blob_value BLOB, large_value INTEGER)",
            &[],
        );
        execute_sql(
            &mut connection,
            "INSERT INTO values_table VALUES (?, ?)",
            &[
                Data::Bytes(vec![0, 255, 128]),
                Data::Integer(MAX_SAFE_INTEGER + 1),
            ],
        );
        let Data::Map(result) = query_sql(
            &mut connection,
            "SELECT blob_value, large_value FROM values_table",
            &[],
        ) else {
            panic!("query response must be a map");
        };
        let Data::Array(rows) = result.get("行").unwrap() else {
            panic!("rows must be an array");
        };
        let Data::Map(row) = &rows[0] else {
            panic!("row must be a map");
        };
        assert_eq!(row.get("blob_value"), Some(&Data::Bytes(vec![0, 255, 128])));
        assert_eq!(
            row.get("large_value"),
            Some(&Data::String((MAX_SAFE_INTEGER + 1).to_string()))
        );
    }

    #[test]
    fn transaction_controls_share_connection_and_quote_savepoint_names() {
        let mut connection = memory_connection();
        execute_sql(
            &mut connection,
            "CREATE TABLE items(value TEXT NOT NULL)",
            &[],
        );

        assert_success(&begin_transaction(&mut connection, "IMMEDIATE"));
        assert!(!connection.is_autocommit());
        execute_sql(
            &mut connection,
            "INSERT INTO items(value) VALUES (?)",
            &[Data::String("keep".into())],
        );
        let savepoint = "safe\"; DROP TABLE items; --";
        assert_success(&savepoint_control(
            &mut connection,
            Operation::Savepoint,
            savepoint,
        ));
        execute_sql(
            &mut connection,
            "INSERT INTO items(value) VALUES (?)",
            &[Data::String("discard".into())],
        );
        assert_success(&savepoint_control(
            &mut connection,
            Operation::RollbackTo,
            savepoint,
        ));
        assert_success(&savepoint_control(
            &mut connection,
            Operation::Release,
            savepoint,
        ));
        assert_success(&finish_transaction(&mut connection, "COMMIT"));
        assert!(connection.is_autocommit());
        assert_eq!(row_count(&mut connection), 1);

        assert_success(&begin_transaction(&mut connection, "DEFERRED"));
        execute_sql(
            &mut connection,
            "INSERT INTO items(value) VALUES (?)",
            &[Data::String("rollback".into())],
        );
        assert_success(&finish_transaction(&mut connection, "ROLLBACK"));
        assert_eq!(row_count(&mut connection), 1);
    }

    #[test]
    fn prepared_statement_reuses_sql_and_enforces_parameter_count() {
        let mut connection = memory_connection();
        execute_sql(
            &mut connection,
            "CREATE TABLE items(value TEXT NOT NULL)",
            &[],
        );
        let statement =
            compile_statement(&mut connection, 17, "INSERT INTO items(value) VALUES (?)").unwrap();
        assert_eq!(statement.parent_connection, 17);
        assert_eq!(statement.parameter_count, 1);

        assert_success(&execute_prepared(
            &mut connection,
            &statement,
            &[Data::String("first".into())],
        ));
        assert_success(&execute_prepared(
            &mut connection,
            &statement,
            &[Data::String("second".into())],
        ));
        assert_eq!(row_count(&mut connection), 2);

        let Data::Map(failure) = execute_prepared(&mut connection, &statement, &[]) else {
            panic!("failure response must be a map");
        };
        assert_eq!(
            failure.get("代码"),
            Some(&Data::String("SQLITE_PARAMETER_COUNT".into()))
        );

        let query = compile_statement(
            &mut connection,
            17,
            "SELECT value FROM items WHERE value = ?",
        )
        .unwrap();
        let Data::Map(result) =
            query_prepared(&mut connection, &query, &[Data::String("second".into())])
        else {
            panic!("query response must be a map");
        };
        let Data::Array(rows) = result.get("行").unwrap() else {
            panic!("rows must be an array");
        };
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn reflects_columns_indexes_and_foreign_keys_with_bound_table_names() {
        let mut connection = memory_connection();
        connection
            .execute_batch(
                "CREATE TABLE parents(id INTEGER PRIMARY KEY);\
                 CREATE TABLE children(\
                     id INTEGER PRIMARY KEY,\
                     parent_id INTEGER REFERENCES parents(id) ON DELETE CASCADE,\
                     code TEXT NOT NULL DEFAULT 'new' UNIQUE\
                 );\
                 CREATE INDEX children_parent_idx \
                     ON children(parent_id DESC) WHERE parent_id IS NOT NULL;",
            )
            .unwrap();

        let Data::Array(tables) = table_list(&mut connection).unwrap() else {
            panic!("table list must be an array");
        };
        assert!(tables.iter().any(|table| {
            matches!(table, Data::Map(table) if table.get("名称") == Some(&Data::String("children".into())))
        }));

        let Data::Map(structure) = table_structure(&mut connection, "children").unwrap() else {
            panic!("table structure must be a map");
        };
        assert_eq!(structure.get("存在"), Some(&Data::Bool(true)));
        assert!(matches!(structure.get("列"), Some(Data::Array(columns)) if columns.len() == 3));
        assert!(matches!(structure.get("索引"), Some(Data::Array(indexes)) if indexes.len() >= 2));
        let Some(Data::Array(foreign_keys)) = structure.get("外键") else {
            panic!("foreign keys must be an array");
        };
        assert_eq!(foreign_keys.len(), 1);
        assert!(matches!(
            &foreign_keys[0],
            Data::Map(key)
                if key.get("目标表") == Some(&Data::String("parents".into()))
                    && key.get("删除动作") == Some(&Data::String("CASCADE".into()))
        ));

        let injected = "children'); DROP TABLE parents; --";
        let Data::Map(missing) = table_structure(&mut connection, injected).unwrap() else {
            panic!("missing table structure must be a map");
        };
        assert_eq!(missing.get("存在"), Some(&Data::Bool(false)));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM parents", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    fn assert_success(value: &Data) {
        let Data::Map(value) = value else {
            panic!("response must be a map");
        };
        assert_eq!(value.get("成功"), Some(&Data::Bool(true)));
    }

    fn row_count(connection: &mut Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap()
    }
}
