# Koral zsh integration (loaded via temporary ZDOTDIR without modifying user dotfiles).

if [[ -n "${KORAL_ZSH_ZDOTDIR+x}" ]]; then
    ZDOTDIR="$KORAL_ZSH_ZDOTDIR"
    unset KORAL_ZSH_ZDOTDIR
else
    unset ZDOTDIR
fi

if [[ -r "${ZDOTDIR:-$HOME}/.zshenv" ]]; then
    source "${ZDOTDIR:-$HOME}/.zshenv"
fi

if [[ -o interactive && -z "${KORAL_SHELL_INTEGRATION:-}" ]]; then
    export KORAL_SHELL_INTEGRATION=1

    __koral_osc7_path() {
        emulate -L zsh
        local LC_ALL=C p="$PWD" out="" i ch hex
        for ((i = 1; i <= ${#p}; i++)); do
            ch="${p[i]}"
            case "$ch" in
                [a-zA-Z0-9._~/-]) out+="$ch" ;;
                *) printf -v hex '%%%02X' "'$ch"; out+="$hex" ;;
            esac
        done
        print -rn -- "$out"
    }

    __koral_precmd() {
        local last_status=$?
        if [[ -n "${__koral_ran:-}" ]]; then
            unset __koral_ran
            printf '\e]133;D;%d\a' "$last_status"
        fi
        printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$(__koral_osc7_path)"
        printf '\e]133;A\a'
        if [[ "${PS1:-}" != *"133;B"* ]]; then
            PS1="${PS1:-}%{\e]133;B\a%}"
        fi
    }

    __koral_preexec() {
        __koral_ran=1
        printf '\e]133;C\a'
    }

    autoload -Uz add-zsh-hook
    add-zsh-hook precmd __koral_precmd
    add-zsh-hook preexec __koral_preexec
fi
