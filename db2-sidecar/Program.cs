// db2-sidecar/Program.cs
using System.Text.Json;
using System.Text.Json.Serialization;
using Db2Sidecar;

var jsonOptions = new JsonSerializerOptions
{
    PropertyNamingPolicy         = JsonNamingPolicy.SnakeCaseLower,
    DefaultIgnoreCondition       = JsonIgnoreCondition.WhenWritingNull,
    PropertyNameCaseInsensitive  = true,
};

var cm = new ConnectionManager();

string? line;
while ((line = Console.ReadLine()) is not null)
{
    if (string.IsNullOrWhiteSpace(line)) continue;
    try
    {
        var req  = JsonSerializer.Deserialize<Request>(line, jsonOptions)!;
        var resp = await CommandHandler.Handle(req, cm);
        Console.WriteLine(JsonSerializer.Serialize(resp, jsonOptions));
    }
    catch (Exception ex)
    {
        // Malformed JSON or unexpected error: emit a safe error response
        Console.WriteLine(JsonSerializer.Serialize(
            new Response { Id = "?", Ok = false, Error = ex.Message },
            jsonOptions));
    }
}
