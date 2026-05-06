package com.aiterm.db2sidecar;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.SQLException;
import java.util.HashMap;
import java.util.Map;

public class ConnectionManager {
    private final Map<String, Connection> connections = new HashMap<>();

    /**
     * Opens a JDBC connection and stores it under connId.
     * connString is a JDBC URL: jdbc:db2://host:port/database
     * Returns null on success, or an error message on failure.
     */
    public String connect(String connId, String connString, String username, String password) {
        try {
            Class.forName("com.ibm.db2.jcc.DB2Driver");
            Connection conn = DriverManager.getConnection(connString, username, password);
            Connection existing = connections.put(connId, conn);
            if (existing != null) {
                try { existing.close(); } catch (SQLException ignored) {}
            }
            return null;
        } catch (ClassNotFoundException e) {
            return "IBM JDBC driver not found: " + e.getMessage();
        } catch (SQLException e) {
            return formatSqlError(e);
        }
    }

    /** Returns the live connection, or null if not found. */
    public Connection get(String connId) {
        return connections.get(connId);
    }

    /** Closes and removes a connection. No-op if not found. */
    public void disconnect(String connId) {
        Connection conn = connections.remove(connId);
        if (conn != null) {
            try { conn.close(); } catch (SQLException ignored) {}
        }
    }

    private String formatSqlError(SQLException e) {
        StringBuilder sb = new StringBuilder(e.getMessage());
        sb.append(" [SQLSTATE=").append(e.getSQLState())
          .append(" ErrorCode=").append(e.getErrorCode()).append("]");
        return sb.toString();
    }
}
