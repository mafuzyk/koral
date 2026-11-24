# Koral bash integration (sourced via --rcfile without modifying user rc files).

if [[ -r /etc/bash.bashrc ]]; then
    source /etc/bash.bashrc
fi
if [[ -r "${HOME}/.bashrc" ]]; then
    source "${HOME}/.bashrc"
fi

[[ $- == *i* ]] || return 0
if [[ "${__koral_integrated:-}" == 1 ]]; then
    return 0
fi
__koral_integrated=1
export KORAL_SHELL_INTEGRATION=1

__koral_nil=""

__koral_osc7_path() {
    local LC_ALL=C path="$PWD" out="" i ch hex
    for ((i = 0; i < ${#path}; i++)); do
        ch="${path:i:1}"
        case "$ch" in
            [a-zA-Z0-9._~/-]) out+="$ch" ;;
            *) printf -v hex '%%%02X' "'$ch"; out+="$hex" ;;
        esac
    done
    printf '%s' "$out"
}

__koral_precmd() {
    local status=$?
    if [[ -n "${__koral_ran:-}" ]]; then
        unset __koral_ran
        printf '\e]133;D;%d\a' "$status"
    fi
    printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$(__koral_osc7_path)"
    printf '\e]133;A\a'
}

if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a "* ]]; then
    PROMPT_COMMAND=(__koral_precmd "${PROMPT_COMMAND[@]}")
elif [[ -n "${PROMPT_COMMAND:-}" ]]; then
    PROMPT_COMMAND="__koral_precmd;${PROMPT_COMMAND}"
else
    PROMPT_COMMAND=__koral_precmd
fi

if [[ "${PS1:-}" != *"133;B"* ]]; then
    PS1="${PS1:-}\[\e]133;B\a\]"
fi

PS0="${PS0:-}\${__koral_nil:\${__koral_ran:=1}:0}\e]133;C\a"
