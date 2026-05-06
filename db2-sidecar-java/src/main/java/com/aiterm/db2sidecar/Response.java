package com.aiterm.db2sidecar;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.List;

@JsonInclude(JsonInclude.Include.NON_NULL)
public class Response {
    public String id = "";
    public boolean ok;
    public String error;
    public List<String> columns;
    public List<List<String>> rows;
    public Long affectedRows;
    public long executionTimeMs;
}
