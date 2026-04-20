// db2-sidecar/CommandHandler.cs
using System.Diagnostics;
using IBM.Data.DB2.Core;

namespace Db2Sidecar;

public static class CommandHandler
{
    public static async Task<Response> Handle(Request req, ConnectionManager cm)
    {
        return req.Cmd switch
        {
            "connect"          => await Connect(req, cm),
            "disconnect"       => Disconnect(req, cm),
            "ping"             => await Ping(req, cm),
            "execute"          => await Execute(req, cm),
            "list_schemas"     => await ListSchemas(req, cm),
            "list_tables"      => await ListTables(req, cm),
            "get_table_schema" => await GetTableSchema(req, cm),
            _                  => new Response { Id = req.Id, Ok = false, Error = $"unknown_cmd:{req.Cmd}" }
        };
    }

    private static async Task<Response> Connect(Request req, ConnectionManager cm)
    {
        var err = await cm.Connect(req.ConnId!, req.ConnString!, req.Username!, req.Password!);
        return err is null
            ? new Response { Id = req.Id, Ok = true }
            : new Response { Id = req.Id, Ok = false, Error = err };
    }

    private static Response Disconnect(Request req, ConnectionManager cm)
    {
        cm.Disconnect(req.ConnId!);
        return new Response { Id = req.Id, Ok = true };
    }

    private static async Task<Response> Ping(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        try
        {
            using var cmd = new DB2Command("SELECT 1 FROM SYSIBM.SYSDUMMY1", conn);
            await cmd.ExecuteScalarAsync();
            return new Response { Id = req.Id, Ok = true };
        }
        catch (Exception ex)
        {
            return new Response { Id = req.Id, Ok = false, Error = ex.Message };
        }
    }

    private static async Task<Response> Execute(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        return await RunSql(req.Id, conn, req.Sql!);
    }

    private static async Task<Response> ListSchemas(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        const string sql =
            "SELECT DISTINCT SCHEMANAME FROM SYSCAT.SCHEMATA " +
            "WHERE DEFINERTYPE = 'U' ORDER BY SCHEMANAME";
        return await RunSql(req.Id, conn, sql);
    }

    private static async Task<Response> ListTables(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        var schema = req.Schema!.Replace("'", "''");
        var sql = $"SELECT TABNAME, TYPE FROM SYSCAT.TABLES WHERE TABSCHEMA = '{schema}' ORDER BY TABNAME";
        return await RunSql(req.Id, conn, sql);
    }

    private static async Task<Response> GetTableSchema(Request req, ConnectionManager cm)
    {
        var conn = cm.Get(req.ConnId!);
        if (conn is null)
            return new Response { Id = req.Id, Ok = false, Error = "conn_not_found" };
        var schema = req.Schema!.Replace("'", "''");
        var table  = req.Table!.Replace("'", "''");
        var sql =
            $"SELECT COLNAME, TYPENAME, DEFAULT, NULLS " +
            $"FROM SYSCAT.COLUMNS " +
            $"WHERE TABSCHEMA = '{schema}' AND TABNAME = '{table}' " +
            $"ORDER BY COLNO";
        return await RunSql(req.Id, conn, sql);
    }

    private static async Task<Response> RunSql(string reqId, DB2Connection conn, string sql)
    {
        var sw = Stopwatch.StartNew();
        try
        {
            using var cmd    = new DB2Command(sql, conn);
            using var reader = await cmd.ExecuteReaderAsync();

            var columns = Enumerable.Range(0, reader.FieldCount)
                .Select(i => reader.GetName(i))
                .ToList();

            var rows = new List<List<string?>>();
            while (await reader.ReadAsync())
            {
                var row = Enumerable.Range(0, reader.FieldCount)
                    .Select(i => reader.IsDBNull(i) ? null : reader.GetValue(i)?.ToString())
                    .ToList();
                rows.Add(row);
            }

            sw.Stop();
            return new Response
            {
                Id = reqId, Ok = true,
                Columns = columns, Rows = rows,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
        catch (Exception ex)
        {
            sw.Stop();
            return new Response
            {
                Id = reqId, Ok = false,
                Error = ex.Message,
                ExecutionTimeMs = sw.ElapsedMilliseconds
            };
        }
    }
}
