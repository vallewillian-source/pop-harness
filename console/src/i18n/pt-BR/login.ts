export const login = {
    title: "Console do POP Harness",
    subtitle: "Acesso administrativo por token. Uso interno.",
    expiredTitle: "Sessão encerrada",
    expiredBody: "O token desta sessão não é mais aceito pela instância.",
    offlineTitle: "Sem conexão",
    invalidToken:
        "Token administrativo inválido ou revogado. Confira com a equipe de operação ou gere outro com `poph token`.",
    unreachableInstance:
        "Não foi possível alcançar a instância em {origin}. Verifique se o processo `poph serve` está em execução.",
    unexpectedError: "Falha inesperada ao entrar. Tente de novo.",
    tokenLabel: "Token administrativo",
    tokenHint: "Gerado com `poph token` na instância, ou definido em [admin].tokens_globais.",
    tokenPlaceholder: "pophg_… ou poph_…",
    showToken: "Mostrar token",
    hideToken: "Ocultar token",
    instanceLabel: "Instância",
    submit: "Entrar no Console",
    sessionNotice:
        "O token fica só na memória desta aba e no armazenamento de sessão do navegador: fechar a aba encerra o acesso. Nada é gravado de forma permanente.",
    scopeNotice:
        "Escopo é definido pelo token: global enxerga todas as organizações, de organização enxerga só a própria.",
    emptyToken: "informe o token administrativo",
} as const satisfies Record<string, string>;
