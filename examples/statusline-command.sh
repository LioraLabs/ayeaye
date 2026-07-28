#!/usr/bin/env bash
input=$(cat)

cwd=$(echo "$input" | jq -r '.workspace.current_dir // .cwd')
model=$(echo "$input" | jq -r '.model.display_name // empty')
used_pct=$(echo "$input" | jq -r '.context_window.used_percentage // empty')
session_id=$(echo "$input" | jq -r '.session_id // empty')

# Path segment: replace $HOME with ~
home="$HOME"
path_segment="${cwd/#$home/\~}"

# Git info
git_info=""
if git -C "$cwd" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    branch=$(git -C "$cwd" branch --show-current 2>/dev/null)
    status_output=$(git -C "$cwd" status --porcelain 2>/dev/null)
    if [ -z "$status_output" ]; then
        status_indicator="\033[32m✓\033[0m"
    else
        status_indicator="\033[31m✗\033[0m"
    fi
    git_info=" \033[37m[\033[36m${branch}\033[0m ${status_indicator}\033[37m]\033[0m"
fi

# Context usage
ctx_info=""
if [ -n "$used_pct" ]; then
    ctx_info=" \033[33mctx:$(printf '%.0f' "$used_pct")%\033[0m"
fi

# Model info
model_info=""
if [ -n "$model" ]; then
    model_info=" \033[35m${model}\033[0m"
fi

# Session marker, for tools that can only see the rendered pane.
#
# ayeaye reads panes with `tmux capture-pane`, which gives text and no
# session context. Printing a short, greppable tag here means anything looking
# at the pane can find the session and open its transcript JSONL directly --
# no guessing from cwd and mtime, which is ambiguous when several agents share
# a directory. Eight hex chars is enough to glob the transcript uniquely.
#
# Dim so it stays out of the way on screen; capture-pane strips the colour, so
# the tag is still plain text to whatever is reading.
# On its own line, and first on that line, deliberately. Appended to the path
# line it gets truncated whenever the working directory is long -- and a
# clipped marker means the transcript view stops working, silently.
printf "\033[34m%s\033[0m%b%b%b\n" "$path_segment" "$git_info" "$model_info" "$ctx_info"
if [ -n "$session_id" ]; then
    printf "\033[90m⟪cc:%s⟫\033[0m\n" "${session_id:0:8}"
fi
