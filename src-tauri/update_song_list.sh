#!/bin/bash

INPUT_FILE="/Users/jamesju/Documents/TEST/SongList.txt"
TEMP_FILE="/Users/jamesju/Documents/TEST/SongList_temp.txt"

# 清空暫存檔案
> "$TEMP_FILE"

# 檢查原始檔案是否存在
if [ ! -f "$INPUT_FILE" ]; then
    echo "錯誤：找不到檔案 $INPUT_FILE"
    exit 1
fi

# 逐行讀取檔案
while IFS= read -r song_name || [ -n "$song_name" ]; do
    # 跳過空行
    if [ -z "$song_name" ]; then
        continue
    fi

    # 對歌名進行 URL Encode (簡單處理，實際可使用 urlencode 工具，這裡假設輸入為純文字)
    # 使用 iTunes API: https://itunes.apple.com/search?term=SONG_NAME&media=song&limit=1
    SEARCH_URL="https://itunes.apple.com/search?term=$(echo "$song_name" | sed 's/ /+/g')&media=song&limit=1"
    
    echo "正在查詢: $song_name ..."

    # 呼叫 API 並解析 JSON
    # 我們嘗試獲取第一個結果的 artistName 和 releaseDate
    RESULT=$(curl -s "$SEARCH_URL" | jq -r '
        if .results | length > 0 then
            .results[0] | "\(.artistName)|\(.releaseDate)"
        else
            "N/A|N/A"
        end
    ')

    # 分離結果
    ARTIST=$(echo "$RESULT" | cut -d'|' -f1)
    DATE=$(echo "$RESULT" | cut -d'|' -f2)

    # 寫入暫存檔案
    echo "${song_name} | ${ARTIST} | ${DATE}" >> "$TEMP_FILE"

    # 避免請求過快，暫停 0.5 秒
    sleep 0.5

done < "$INPUT_FILE"

# 將結果覆蓋回原始檔案
mv "$TEMP_FILE" "$INPUT_FILE"

echo "更新完成！結果已寫入 $INPUT_FILE"
