import { invoke } from "@tauri-apps/api/core";

export type DbType = "postgresql" | "mysql" | "sqlite" | "mssql" | "db2";

export interface DbConnectionInfo {
  id: string;
  name: string;
  db_type: DbType;
  host: string;
  port: number;
  database: string;
  username: string;
  default_schema?: string | null;
  is_connected: boolean;
}

export interface DbConnectionInput {
  id?: string;
  name: string;
  db_type: DbType;
  host: string;
  port: number;
  database: string;
  username: string;
  password: string;
  default_schema?: string | null;
}

export interface TableInfo {
  name: string;
  table_type: "table" | "view";
}

export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
  default: string | null;
}

export interface QueryResult {
  columns: string[];
  rows: (string | number | boolean | null)[][];
  affected_rows: number | null;
  execution_time_ms: number;
  error: string | null;
}

export const DB_TYPE_LABELS: Record<DbType, string> = {
  postgresql: "PostgreSQL",
  mysql: "MySQL",
  sqlite: "SQLite",
  mssql: "MSSQL",
  db2: "DB2",
};

export const DB_DEFAULT_PORTS: Record<DbType, number> = {
  postgresql: 5432,
  mysql: 3306,
  sqlite: 0,
  mssql: 1433,
  db2: 50000,
};

export function dbListConnections(): Promise<DbConnectionInfo[]> {
  return invoke("db_list_connections");
}

export function dbAddConnection(input: DbConnectionInput): Promise<string> {
  return invoke("db_add_connection", { input });
}

export function dbUpdateConnection(input: DbConnectionInput): Promise<void> {
  return invoke("db_update_connection", { input });
}

export function dbRemoveConnection(id: string): Promise<void> {
  return invoke("db_remove_connection", { id });
}

export function dbTestConnection(input: DbConnectionInput): Promise<void> {
  return invoke("db_test_connection", { input });
}

export function dbConnect(id: string): Promise<void> {
  return invoke("db_connect", { id });
}

export function dbDisconnect(id: string): Promise<void> {
  return invoke("db_disconnect", { id });
}

export function dbListSchemas(connectionId: string): Promise<string[]> {
  return invoke("db_list_schemas", { connectionId });
}

export function dbListTables(connectionId: string, schema: string): Promise<TableInfo[]> {
  return invoke("db_list_tables", { connectionId, schema });
}

export function dbGetTableSchema(connectionId: string, schema: string, table: string): Promise<ColumnInfo[]> {
  return invoke("db_get_table_schema", { connectionId, schema, table });
}

export function dbExecuteQuery(connectionId: string, sql: string, schema?: string): Promise<QueryResult> {
  return invoke("db_execute_query", { connectionId, sql, schema });
}

export function dbPreviewTable(connectionId: string, schema: string, table: string, page: number, pageSize: number): Promise<QueryResult> {
  return invoke("db_preview_table", { connectionId, schema, table, page, pageSize });
}
