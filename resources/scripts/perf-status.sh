#!/usr/bin/env bash
# leaf icon for the bar w the current power mode in the tooltip
mode=$(powerprofilesctl get 2>/dev/null || echo unknown)
printf '{"text": "󰌪", "tooltip": "Performance · %s", "class": "%s"}\n' "$mode" "$mode"
