# SwiftRef API Java 呼叫範例 - 系統設計文件 (SDD)

## 1. 系統架構概述

本範例採用分層架構，將認證、API 呼叫、資料處理等關注點分離，確保程式碼易讀且可維護。

### 1.1 架構圖

```mermaid
graph TB
    subgraph "應用層"
        A[Main.java<br/>應用程式入口]
    end
    
    subgraph "服務層"
        B[SwiftRefClient.java<br/>API 客戶端]
        C[AuthService.java<br/>認證服務]
    end
    
    subgraph "資料層"
        D[ConfigManager.java<br/>設定管理]
        E[BicResponse.java<br/>回應資料模型]
    end
    
    subgraph "外部系統"
        F[SWIFT OAuth Server]
        G[SWIFT SwiftRef API]
    end
    
    A --> B
    A --> D
    B --> C
    B --> E
    C --> D
    C --> F
    B --> G
    
    style A fill:#e1f5ff
    style B fill:#fff4e1
    style C fill:#fff4e1
    style D fill:#f0f0f0
    style E fill:#f0f0f0
    style F fill:#ffe1e1
    style G fill:#ffe1e1
```

### 1.2 類別關係圖

```mermaid
classDiagram
    class Main {
        +main(String[] args)
    }
    
    class SwiftRefClient {
        -AuthService authService
        -HttpClient httpClient
        -String apiBaseUrl
        +BicResponse queryBic(String bic)
        -String getAccessToken()
    }
    
    class AuthService {
        -ConfigManager config
        -HttpClient httpClient
        -String tokenEndpoint
        +String authenticate()
    }
    
    class ConfigManager {
        -Properties properties
        +String getConsumerKey()
        +String getConsumerSecret()
        +String getApiBaseUrl()
        +String getTokenEndpoint()
    }
    
    class BicResponse {
        -String bic
        -String institutionName
        -String city
        -String country
        +String toString()
    }
    
    Main --> SwiftRefClient
    Main --> ConfigManager
    SwiftRefClient --> AuthService
    SwiftRefClient --> BicResponse
    AuthService --> ConfigManager
```

## 2. 模組設計

### 2.1 Main.java (應用程式入口)
**職責**: 
- 程式入口點
- 初始化元件
- 處理命令列參數
- 呼叫 API 並顯示結果

**流程**:
```mermaid
flowchart TD
    Start([程式啟動]) --> LoadConfig[載入設定]
    LoadConfig --> CheckArgs{檢查參數}
    CheckArgs -->|無參數| UseDefault[使用預設 BIC]
    CheckArgs -->|有參數| UseInput[使用輸入 BIC]
    UseDefault --> CreateClient[建立 SwiftRefClient]
    UseInput --> CreateClient
    CreateClient --> CallAPI[呼叫 queryBic]
    CallAPI --> Success{成功?}
    Success -->|是| Display[顯示結果]
    Success -->|否| Error[顯示錯誤]
    Display --> End([結束])
    Error --> End
```

### 2.2 SwiftRefClient.java (API 客戶端)
**職責**:
- 封裝所有 SwiftRef API 呼叫
- 管理 HTTP 請求/回應
- 處理 JSON 序列化/反序列化

**核心方法**:
```java
public BicResponse queryBic(String bic) throws IOException
```

**錯誤處理**:
- HTTP 4xx/5xx 錯誤
- 網路連線異常
- JSON 解析錯誤

### 2.3 AuthService.java (認證服務)
**職責**:
- 實作 OAuth 2.0 Client Credentials Flow
- 管理 access token 的取得
- 處理認證相關錯誤

**認證流程**:
```mermaid
sequenceDiagram
    participant Client as AuthService
    participant OAuth as SWIFT OAuth Server
    
    Client->>OAuth: POST /oauth2/v1/token
    Note right of Client: grant_type=client_credentials<br/>client_id=xxx<br/>client_secret=xxx
    
    alt 認證成功
        OAuth-->>Client: 200 OK
        Note left of OAuth: {"access_token":"...", "expires_in":3600}
        Client->>Client: 解析 token
    else 認證失敗
        OAuth-->>Client: 401 Unauthorized
        Client->>Client: 拋出例外
    end
```

### 2.4 ConfigManager.java (設定管理)
**職責**:
- 載入設定檔 (config.properties)
- 提供環境變數覆寫機制
- 驗證必要設定項目

**設定優先順序**:
1. 環境變數 (最高優先)
2. config.properties 檔案
3. 預設值 (如果有)

### 2.5 BicResponse.java (資料模型)
**職責**:
- 對應 API 回應的 JSON 結構
- 提供資料存取方法

**JSON 對應範例**:
```json
{
  "bic": "CHASUS33",
  "institution_name": "JPMORGAN CHASE BANK, N.A.",
  "city": "NEW YORK",
  "country": "US"
}
```

## 3. API 呼叫流程

```mermaid
sequenceDiagram
    participant User as 使用者
    participant Main
    participant Client as SwiftRefClient
    participant Auth as AuthService
    participant API as SWIFT API
    
    User->>Main: 執行程式(BIC碼)
    Main->>Client: queryBic("CHASUS33")
    Client->>Auth: authenticate()
    Auth->>API: POST /oauth2/v1/token
    API-->>Auth: access_token
    Auth-->>Client: token
    Client->>API: GET /v1/bicplus/CHASUS33<br/>Authorization: Bearer {token}
    API-->>Client: BIC 資料 (JSON)
    Client->>Client: 解析 JSON → BicResponse
    Client-->>Main: BicResponse 物件
    Main->>Main: 格式化輸出
    Main-->>User: 顯示結果
```

## 4. 專案結構

```
swiftref-example/
├── pom.xml
├── config.properties.example
├── README.md
└── src/
    └── main/
        └── java/
            └── com/
                └── example/
                    └── swiftref/
                        ├── Main.java
                        ├── SwiftRefClient.java
                        ├── AuthService.java
                        ├── ConfigManager.java
                        └── model/
                            └── BicResponse.java
```

## 5. 依賴項目 (Maven)

### 5.1 完整的 pom.xml