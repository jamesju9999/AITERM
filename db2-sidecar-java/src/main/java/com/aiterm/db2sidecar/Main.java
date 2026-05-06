package com.aiterm.db2sidecar;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;

public class Main {
    public static void main(String[] args) throws Exception {
        ObjectMapper mapper = new ObjectMapper()
            .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
            .setSerializationInclusion(JsonInclude.Include.NON_NULL)
            .configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);

        ConnectionManager cm = new ConnectionManager();

        BufferedReader in = new BufferedReader(
            new InputStreamReader(System.in, StandardCharsets.UTF_8));
        PrintStream out = new PrintStream(System.out, true, StandardCharsets.UTF_8);

        String line;
        while ((line = in.readLine()) != null) {
            if (line.isBlank()) continue;
            try {
                Request req = mapper.readValue(line, Request.class);
                Response resp = CommandHandler.handle(req, cm);
                out.println(mapper.writeValueAsString(resp));
            } catch (Exception ex) {
                Response err = new Response();
                err.id = "?";
                err.ok = false;
                err.error = ex.getMessage();
                out.println(mapper.writeValueAsString(err));
            }
        }
    }
}
