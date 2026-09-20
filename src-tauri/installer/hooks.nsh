; AITerm 的 NSIS 安裝程式 hook（Tauri `bundle.windows.nsis.installerHooks`）：
; 在檔案總管的右鍵選單加入「在 AITerm 開啟」（資料夾、資料夾空白處、磁碟機）。
; 註冊在 SHCTX：currentUser 安裝寫進目前使用者，perMachine 安裝寫進本機；解除安裝時移除。
; 更新（/UPDATE）會先跑舊版的解除安裝 hook 再跑新版的安裝 hook，最終狀態一致。
; Windows 11 的新式右鍵選單要按「顯示其他選項」才看得到（傳統 shell verb 的限制）。
; 這個檔案必須存成 UTF-8 with BOM，否則 NSIS（Unicode）讀不對下面的中文。

Var AITermMenuLabel
Var AITermUiLang

; KEY = Software\Classes 底下的位置；ARG = 交給 AITerm 的路徑（資料夾用 %1，資料夾空白處用 %V）。
!macro AITERM_ADD_VERB KEY ARG
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "" "$AITermMenuLabel"
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
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

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey SHCTX "Software\Classes\Directory\shell\AITerm"
  DeleteRegKey SHCTX "Software\Classes\Directory\Background\shell\AITerm"
  DeleteRegKey SHCTX "Software\Classes\Drive\shell\AITerm"
!macroend
