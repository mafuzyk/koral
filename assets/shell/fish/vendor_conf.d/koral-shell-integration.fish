# Koral fish integration (loaded via XDG_DATA_DIRS without modifying user config).

status is-interactive; or exit 0

if set -q XDG_DATA_DIRS
    set -l cleaned
    for dir in $XDG_DATA_DIRS
        if not string match -q '*/koral/shell-integration/fish-data' -- "$dir"; and not string match -q '*/koral-shell-*/fish-data' -- "$dir"
            set -a cleaned "$dir"
        end
    end
    set -gx XDG_DATA_DIRS $cleaned
end

set -q KORAL_SHELL_INTEGRATION; and exit 0
set -gx KORAL_SHELL_INTEGRATION 1

function __koral_preexec --on-event fish_preexec
    set -g __koral_ran 1
    printf '\e]133;C\a'
end

function __koral_postexec --on-event fish_postexec
    set -g __koral_last_status $status
end

function __koral_prompt --on-event fish_prompt
    if set -q __koral_ran
        set -l code 0
        if set -q __koral_last_status
            set code $__koral_last_status
        end
        set -e __koral_ran
        set -e __koral_last_status
        printf '\e]133;D;%d\a' "$code"
    end
    set -l encoded (string escape --style=url -- "$PWD" | string replace -a '%2F' '/')
    printf '\e]7;file://%s%s\a' (hostname) "$encoded"
    printf '\e]133;A\a'
    printf '\e]133;B\a'
end
