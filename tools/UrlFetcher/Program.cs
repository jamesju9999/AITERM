using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Security;
using System.Security.Authentication;
using System.Threading.Tasks;

namespace WebFetcher;

public class Program
{
    private const int DefaultTimeoutSeconds = 60;
    private const int MaxRetries = 2;

    // Python script that uses curl_cffi to impersonate Chrome TLS fingerprint
    private const string CurlCffiScript = """
import sys, json
from curl_cffi import requests as cffi_requests
url = sys.argv[1]
timeout = int(sys.argv[2]) if len(sys.argv) > 2 else 60
r = cffi_requests.get(url, impersonate="chrome120", timeout=timeout)
print(json.dumps({"status": r.status_code, "content": r.text}))
""";

    public static async Task<int> Main(string[] args)
    {
        if (args.Length == 0)
        {
            DisplayHelp();
            return 0;
        }

        if (args.Contains("-h") || args.Contains("--help"))
        {
            DisplayHelp();
            return 0;
        }

        var url = "";
        var silentMode = false;
        var outputFile = "";
        var timeoutSeconds = DefaultTimeoutSeconds;

        for (int i = 0; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "-u":
                case "--url":
                    if (i + 1 < args.Length)
                    {
                        url = args[i + 1];
                        i++;
                    }
                    break;
                case "-s":
                case "--silent":
                    silentMode = true;
                    break;
                case "-o":
                case "--output":
                    if (i + 1 < args.Length)
                    {
                        outputFile = args[i + 1];
                        i++;
                    }
                    break;
                case "-t":
                case "--timeout":
                    if (i + 1 < args.Length && int.TryParse(args[i + 1], out var t) && t > 0)
                    {
                        timeoutSeconds = t;
                        i++;
                    }
                    break;
            }
        }

        if (string.IsNullOrEmpty(url))
        {
            Console.Error.WriteLine("錯誤：必須指定網址 (-u 或 --url)");
            DisplayHelp();
            return 1;
        }

        try
        {
            string content;
            HttpStatusCode statusCode;

            // Try standard HttpClient first; fall back to Chrome TLS impersonation if blocked
            try
            {
                var response = await FetchUrlContentWithRetry(url, timeoutSeconds, silentMode);
                content = response.Content;
                statusCode = response.StatusCode;
            }
            catch (Exception httpEx) when (httpEx is TaskCanceledException || httpEx is HttpRequestException)
            {
                if (!silentMode)
                    Console.Error.WriteLine($"HttpClient 失敗，嘗試 Chrome TLS 模擬模式...");

                var impersonated = await FetchWithChromeImpersonation(url, timeoutSeconds);
                content = impersonated.Content;
                statusCode = impersonated.StatusCode;
            }

            if (!silentMode)
                Console.WriteLine($"\n狀態碼：{statusCode}");

            if (!string.IsNullOrEmpty(outputFile))
            {
                System.IO.File.WriteAllText(outputFile, content);
                Console.WriteLine($"內容已儲存到檔案：{outputFile}");
            }
            else if (!silentMode)
            {
                Console.WriteLine("\n===== HTML 內容 =====");
                Console.WriteLine(content);
            }
            else
            {
                Console.WriteLine(content);
            }

            return 0;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"錯誤：{ex.Message}");
            return 1;
        }
    }

    private static void DisplayHelp()
    {
        Console.WriteLine("""
================================
        WebFetcher 工具說明
================================

用途：抓取指定網址的內容並顯示

用法:
  WebFetcher [選項]

選項:
  -u, --url <URL>       指定要抓取的網址（必需）
  -s, --silent          靜默模式，不顯示狀態資訊
  -o, --output <FILE>   將結果儲存到檔案
  -t, --timeout <SEC>   逾時秒數（預設 60）
  -h, --help            顯示說明

範例:
  WebFetcher -u https://www.example.com
  WebFetcher --url "https://www.google.com" --silent
  WebFetcher -u https://example.com -o result.txt
  WebFetcher -u https://example.com -t 90

回傳碼:
  0  - 成功
  1  - 失敗或錯誤

================================
""");
    }

    private static HttpClient BuildClient(int timeoutSeconds)
    {
        var handler = new SocketsHttpHandler
        {
            AllowAutoRedirect = true,
            MaxAutomaticRedirections = 10,
            AutomaticDecompression = DecompressionMethods.All,
            UseCookies = true,
            CookieContainer = new CookieContainer(),
            UseProxy = true,
            PooledConnectionLifetime = TimeSpan.FromMinutes(2),
            SslOptions = new SslClientAuthenticationOptions
            {
                EnabledSslProtocols = SslProtocols.Tls12 | SslProtocols.Tls13
            }
        };

        var client = new HttpClient(handler);
        client.Timeout = TimeSpan.FromSeconds(timeoutSeconds);

        // 現代瀏覽器 User-Agent
        client.DefaultRequestHeaders.UserAgent.ParseAdd(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");

        // 模擬真實瀏覽器請求標頭
        client.DefaultRequestHeaders.Add("Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8");
        client.DefaultRequestHeaders.Add("Accept-Language", "en-US,en;q=0.9,zh-TW;q=0.8,zh;q=0.7");
        client.DefaultRequestHeaders.Add("Accept-Encoding", "gzip, deflate, br");
        client.DefaultRequestHeaders.Add("Cache-Control", "no-cache");
        client.DefaultRequestHeaders.Add("Sec-Fetch-Dest", "document");
        client.DefaultRequestHeaders.Add("Sec-Fetch-Mode", "navigate");
        client.DefaultRequestHeaders.Add("Sec-Fetch-Site", "none");
        client.DefaultRequestHeaders.Add("Sec-Fetch-User", "?1");
        client.DefaultRequestHeaders.Add("Upgrade-Insecure-Requests", "1");

        return client;
    }

    private static async Task<(HttpStatusCode StatusCode, string Content)> FetchUrlContentWithRetry(
        string url, int timeoutSeconds, bool silentMode)
    {
        // 自動補 https://
        if (!url.StartsWith("http://", StringComparison.OrdinalIgnoreCase) &&
            !url.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
        {
            url = "https://" + url;
            if (!silentMode)
                Console.WriteLine($"自動添加 https:// 前綴：{url}");
        }

        Exception? lastException = null;

        for (int attempt = 0; attempt <= MaxRetries; attempt++)
        {
            try
            {
                if (attempt > 0 && !silentMode)
                {
                    Console.Error.WriteLine($"第 {attempt + 1} 次重試...");
                }

                using var client = BuildClient(timeoutSeconds);
                var response = await client.GetAsync(url);
                var content = await response.Content.ReadAsStringAsync();

                return (response.StatusCode, content);
            }
            catch (Exception ex) when (attempt < MaxRetries &&
                (ex is HttpRequestException || ex is TaskCanceledException))
            {
                lastException = ex;
                await Task.Delay(1000 * (attempt + 1));
            }
        }

        throw lastException ?? new HttpRequestException("請求失敗");
    }

    private static async Task<(HttpStatusCode StatusCode, string Content)> FetchWithChromeImpersonation(
        string url, int timeoutSeconds)
    {
        var scriptPath = System.IO.Path.GetTempFileName() + ".py";
        try
        {
            await System.IO.File.WriteAllTextAsync(scriptPath, CurlCffiScript);

            using var process = new System.Diagnostics.Process();
            process.StartInfo = new System.Diagnostics.ProcessStartInfo
            {
                FileName = "python3",
                Arguments = $"\"{scriptPath}\" \"{url}\" {timeoutSeconds}",
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false
            };

            process.Start();
            var output = await process.StandardOutput.ReadToEndAsync();
            await process.WaitForExitAsync();

            if (process.ExitCode != 0)
            {
                var err = await process.StandardError.ReadToEndAsync();
                throw new HttpRequestException($"Chrome 模擬失敗：{err}");
            }

            var json = System.Text.Json.JsonDocument.Parse(output).RootElement;
            var status = (HttpStatusCode)json.GetProperty("status").GetInt32();
            var content = json.GetProperty("content").GetString() ?? "";
            return (status, content);
        }
        finally
        {
            System.IO.File.Delete(scriptPath);
        }
    }
}
