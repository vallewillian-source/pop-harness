export const routing = {
    validationHumanDepartment:
        "escolha o departamento humano de destino para ligar a transferência",
    validationTooManyReasons: "use no máximo {limit} motivos (há {count})",
    validationReasonTooLong: "o motivo aceita até {limit} caracteres",
    departmentInactive: "{name} (inativo)",
    departmentNotFound: "departamento não encontrado na organização",
    loadAgentFailed: "falha ao carregar o agente",
    loadDepartmentsFailed: "falha ao carregar os departamentos",
    nothingToSave: "Nada para gravar: o roteamento já está como está salvo.",
    savedVersion: "Roteamento gravado (versão {version}).",
    conflictMessage:
        "Outra pessoa salvou este agente antes de você. O que você digitou foi mantido: revise e salve de novo.",
    conflictMessageVersion:
        "Outra pessoa salvou este agente antes de você (agora na versão {version}). O que você digitou foi mantido: revise e salve de novo.",
    errorWithDetails: "{message}: {reason} (código {code})",
    errorWithCode: "{message} (código {code})",
    unexpectedSaveError: "falha inesperada ao salvar o roteamento",
    loadErrorTitle: "Não consegui carregar o roteamento do agente",
    loadErrorDescription:
        "A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor.",
    retry: "Tentar de novo",
    headerDescription: "para onde a conversa vai e quando passa a humano",
    conflictTitle: "Versão desatualizada",
    saveErrorTitle: "Não foi possível salvar",
    departmentsErrorTitle: "Não consegui carregar os departamentos",
    sectionDepartments: "Departamentos do agente",
    sectionDepartmentsDescription:
        "Os departamentos são da organização (criados em Estrutura). Aqui você escolhe quais o agente usa.",
    emptyDepartmentsTitle: "Nenhum departamento na organização",
    emptyDepartmentsDescription:
        "Crie os departamentos em Estrutura antes de escolher o departamento padrão e o de transferência.",
    goToStructure: "Ir para Estrutura",
    defaultDepartmentLabel: "Departamento padrão",
    defaultDepartmentHint:
        "Onde a conversa deste agente começa. Só dá para trocar; a API não aceita deixar sem departamento.",
    chooseDepartment: "Escolha um departamento",
    handoffLabel: "Transferir para atendente humano",
    handoffDescription:
        "Liga a transferência: quando o motor decidir que o caso é de humano, a conversa vai para o departamento de destino abaixo.",
    humanDepartmentLabel: "Departamento humano de destino",
    humanDepartmentHint:
        "Para onde a conversa vai quando há transferência. Obrigatório com a transferência ligada.",
    sectionReasons: "Motivos de transferência",
    sectionReasonsDescription:
        "Situações em que a conversa deve passar a humano, em linguagem simples (ex.: “cliente pede segunda via”). Até {limit} motivos, com até {maxLength} caracteres cada. Motivos vazios não são gravados.",
    addReason: "Adicionar motivo",
    noReasons:
        "Nenhum motivo cadastrado. Sem motivos, o agente só transfere nos casos de risco que o próprio motor já trata.",
    reasonLabel: "Motivo {number}",
    removeReasonLabel: "Remover motivo {number}",
    unsavedChanges: "Alterações não salvas",
    savedStatus: "Salvo (versão {version})",
    discardChanges: "Descartar alterações",
    saveRouting: "Salvar roteamento",
    discarded: "Alterações descartadas: voltou ao que está salvo.",
} as const satisfies Record<string, string>;
