package com.aiterm.db2sidecar;

import java.sql.*;
import java.util.ArrayList;
import java.util.List;

public class CommandHandler {

    public static Response handle(Request req, ConnectionManager cm) throws Exception {
        switch (req.cmd) {
            case "connect":          return connect(req, cm);
            case "disconnect":       return disconnect(req, cm);
            case "ping":             return ping(req, cm);
            case "execute":          return execute(req, cm);
            case "list_schemas":     return listSchemas(req, cm);
            case "list_tables":      return listTables(req, cm);
            case "get_table_schema": return getTableSchema(req, cm);
            default:
                Response r = new Response();
                r.id = req.id;
                r.ok = false;
                r.error = "unknown_cmd:" + req.cmd;
                return r;
        }
    }

    private static Response connect(Request req, ConnectionManager cm) {
        String err = cm.connect(req.connId, req.connString, req.username, req.password);
        Response r = new Response();
        r.id = req.id;
        r.ok = (err == null);
        r.error = err;
        return r;
    }

    private static Response disconnect(Request req, ConnectionManager cm) {
        cm.disconnect(req.connId);
        Response r = new Response();
        r.id = req.id;
        r.ok = true;
        return r;
    }

    private static Response ping(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        try {
            try (Statement stmt = conn.createStatement()) {
                stmt.executeQuery("SELECT 1 FROM SYSIBM.SYSDUMMY1").close();
            }
            Response r = new Response();
            r.id = req.id;
            r.ok = true;
            return r;
        } catch (SQLException e) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = e.getMessage();
            return r;
        }
    }

    private static Response execute(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        return runSql(req.id, conn, req.sql);
    }

    private static Response listSchemas(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        String sql = "SELECT DISTINCT SCHEMANAME FROM SYSCAT.SCHEMATA " +
                     "WHERE DEFINERTYPE = 'U' ORDER BY SCHEMANAME";
        return runSql(req.id, conn, sql);
    }

    private static Response listTables(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        String schema = req.schema.replace("'", "''");
        String sql = "SELECT TABNAME, TYPE FROM SYSCAT.TABLES " +
                     "WHERE TABSCHEMA = '" + schema + "' ORDER BY TABNAME";
        return runSql(req.id, conn, sql);
    }

    private static Response getTableSchema(Request req, ConnectionManager cm) {
        Connection conn = cm.get(req.connId);
        if (conn == null) {
            Response r = new Response();
            r.id = req.id;
            r.ok = false;
            r.error = "conn_not_found";
            return r;
        }
        String schema = req.schema.replace("'", "''");
        String table  = req.table.replace("'", "''");
        String sql = "SELECT COLNAME, TYPENAME, DEFAULT, NULLS " +
                     "FROM SYSCAT.COLUMNS " +
                     "WHERE TABSCHEMA = '" + schema + "' AND TABNAME = '" + table + "' " +
                     "ORDER BY COLNO";
        return runSql(req.id, conn, sql);
    }

    private static Response runSql(String reqId, Connection conn, String sql) {
        long start = System.currentTimeMillis();
        try (Statement stmt = conn.createStatement()) {
            boolean hasResultSet = stmt.execute(sql);
            long elapsed = System.currentTimeMillis() - start;
            Response r = new Response();
            r.id = reqId;
            r.ok = true;
            r.executionTimeMs = elapsed;
            if (hasResultSet) {
                try (ResultSet rs = stmt.getResultSet()) {
                    ResultSetMetaData meta = rs.getMetaData();
                    int colCount = meta.getColumnCount();
                    List<String> columns = new ArrayList<>(colCount);
                    for (int i = 1; i <= colCount; i++) {
                        columns.add(meta.getColumnName(i));
                    }
                    List<List<String>> rows = new ArrayList<>();
                    while (rs.next()) {
                        List<String> row = new ArrayList<>(colCount);
                        for (int i = 1; i <= colCount; i++) {
                            String val = rs.getString(i);
                            row.add(rs.wasNull() ? null : val);
                        }
                        rows.add(row);
                    }
                    r.columns = columns;
                    r.rows = rows;
                }
            } else {
                r.affectedRows = (long) stmt.getUpdateCount();
            }
            return r;
        } catch (SQLException e) {
            long elapsed = System.currentTimeMillis() - start;
            Response r = new Response();
            r.id = reqId;
            r.ok = false;
            r.error = e.getMessage();
            r.executionTimeMs = elapsed;
            return r;
        }
    }
}
