namespace Db2Sidecar;

public class Request
{
    public string Id { get; set; } = "";
    public string Cmd { get; set; } = "";
    public string? ConnId { get; set; }
    public string? ConnString { get; set; }
    public string? Username { get; set; }
    public string? Password { get; set; }
    public string? Sql { get; set; }
    public string? Schema { get; set; }
    public string? Table { get; set; }
}

public class Response
{
    public string Id { get; set; } = "";
    public bool Ok { get; set; }
    public string? Error { get; set; }
    public List<string>? Columns { get; set; }
    public List<List<string?>>? Rows { get; set; }
    public long? AffectedRows { get; set; }
    public long ExecutionTimeMs { get; set; }
}
