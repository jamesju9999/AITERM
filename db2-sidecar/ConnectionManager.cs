// db2-sidecar/ConnectionManager.cs
using IBM.Data.DB2.Core;

namespace Db2Sidecar;

public class ConnectionManager
{
    private readonly Dictionary<string, DB2Connection> _connections = new();

    /// <summary>
    /// Opens a DB2 connection and stores it under connId.
    /// Returns null on success, or an error message on failure.
    /// </summary>
    public async Task<string?> Connect(string connId, string connString, string username, string password)
    {
        try
        {
            // IBM.Data.Db2.Core accepts UID/PWD in the connection string
            var fullConnStr = $"{connString};UID={username};PWD={password};";
            var conn = new DB2Connection(fullConnStr);
            await conn.OpenAsync();
            _connections[connId] = conn;
            return null;
        }
        catch (Exception ex)
        {
            return ex.Message;
        }
    }

    /// <summary>Returns the live connection, or null if not found.</summary>
    public DB2Connection? Get(string connId) =>
        _connections.TryGetValue(connId, out var conn) ? conn : null;

    /// <summary>Closes and removes a connection. No-op if not found.</summary>
    public void Disconnect(string connId)
    {
        if (_connections.Remove(connId, out var conn))
        {
            conn.Close();
            conn.Dispose();
        }
    }
}
