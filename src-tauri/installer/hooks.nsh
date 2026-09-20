; AITerm 的 NSIS 安裝程式 hook（Tauri `bundle.windows.nsis.installerHooks`）：
; 在檔案總管的右鍵選單加入「在 AITerm 開啟」（資料夾、資料夾空白處、磁碟機）。
; 註冊在 SHCTX：currentUser 安裝寫進目前使用者，perMachine 安裝寫進本機；解除安裝時移除。
; 移除放在 POSTUNINSTALL（不是 PREUNINSTALL）：Tauri 的 PREUNINSTALL 在「應用程式執行中」的檢查之前就跑，
; 使用者在那一步取消的話，程式還裝著、選單卻已經被刪掉了；POSTUNINSTALL 只在真的解除安裝時才會跑到。
; 更新：靜默更新（/UPDATE）時 Tauri 的樣板會略過舊版的解除安裝，只跑 POSTINSTALL 覆寫同樣的鍵；
; 互動式「先解除安裝舊版再安裝」的路徑才會先跑舊版的解除安裝 hook，再跑 POSTINSTALL。兩條路最終狀態都一致。
; Windows 11 的新式右鍵選單要按「顯示其他選項」才看得到（傳統 shell verb 的限制）。
; 這個檔案存成 UTF-8 with BOM：BOM 是 NSIS 唯一明確的編碼宣告，沒有 BOM 的檔案會依 ANSI 字碼頁轉換
; （除非指定 /charset），而下面有中文，所以不能省。POSIX 版的 makensis 會把沒有 BOM 的檔案當 UTF-8 讀，
; 所以在 macOS／Linux 上編譯看不出差別，只有靜態測試與 Windows 上的實際建置能把關。

Var AITermMenuLabel
Var AITermUiLang

; KEY = Software\Classes 底下的位置；ARG = 交給 AITerm 的路徑（資料夾用 %1，資料夾空白處用 %V）。
!macro AITERM_ADD_VERB KEY ARG
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "" "$AITermMenuLabel"
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  ; NoWorkingDirectory：不然檔案總管會把被點的資料夾當成新行程的 cwd，第一個實例會握著它到結束，
  ; AITerm 開著時那個資料夾就刪不掉、改不了名（我們傳的都是絕對路徑，不依賴 cwd）。
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "NoWorkingDirectory" ""
  ; MultiSelectModel=Single：預設會對每個被選取的資料夾各啟動一個行程，AITerm 還沒執行時會跟單一實例外掛搶；
  ; Single 讓多選時不顯示這一項。
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "MultiSelectModel" "Single"
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"${ARG}$\""
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; 安裝程式本身只有英文，所以用作業系統的 UI 語言決定選單文字：繁體中文（台灣／香港／澳門）用中文。
  StrCpy $AITermMenuLabel "Open in AITerm"
  ; 回傳值走堆疊再 Pop 進自己的變數：hook 是插在 Tauri 的 installer.nsi 裡跑的，不可動共用暫存器 $0–$9／$R0–$R9。
  System::Call 'kernel32::GetUserDefaultUILanguage() i .s'
  Pop $AITermUiLang
  ${If} $AITermUiLang = 0x0404
  ${OrIf} $AITermUiLang = 0x0C04
  ${OrIf} $AITermUiLang = 0x1404
    StrCpy $AITermMenuLabel "在 AITerm 開啟"
  ${EndIf}
  !insertmacro AITERM_ADD_VERB "Directory" "%1"
  !insertmacro AITERM_ADD_VERB "Directory\Background" "%V"
  !insertmacro AITERM_ADD_VERB "Drive" "%1"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey SHCTX "Software\Classes\Directory\shell\AITerm"
  DeleteRegKey SHCTX "Software\Classes\Directory\Background\shell\AITerm"
  DeleteRegKey SHCTX "Software\Classes\Drive\shell\AITerm"
!macroend
