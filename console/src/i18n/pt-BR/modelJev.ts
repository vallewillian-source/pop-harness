export const modelJev = {
    gateG1Title: "G1 · Transferir para humano agora?",
    gateG1Description:
        "Decide se a conversa vai para atendente humano. Inclui a orientação de risco à saúde, que depende da política da empresa.",
    gateG2Title: "G2 · Esperar mais mensagens ou responder agora?",
    gateG2Description:
        "Ajusta a espera antes de responder quando o cliente manda mensagens em sequência.",
    gateG3Title: "G3 · Precisa de base, de produto ou de nada?",
    gateG3Description:
        "Em saudação e agradecimento, dispensa a consulta à base e ao catálogo.",
    gateG4Title: "G4 · Qual departamento recebe a transferência?",
    gateG4Description:
        "Escolhe um dos departamentos ativos da organização quando há transferência. Sem escolha válida, vale o departamento padrão do agente.",
    gateG5Title: "G5 · Qual produto o cliente quer?",
    gateG5Description:
        "Escolhe o produto entre os candidatos do catálogo quando há mais de um possível. Ligá-lo faz a busca de produtos rodar em todo turno.",
    gateG6Title: "G6 · A resposta pode ser enviada?",
    gateG6Description:
        "Revisa a resposta antes do envio. Se reprovar, o modelo reescreve uma vez; se reprovar de novo, sai a resposta segura do agente.",
    gateG7Title: "G7 · É injeção de prompt ou abuso?",
    gateG7Description:
        "Identifica tentativa de dar ordens ao assistente ou ofensa, na mensagem e no texto de arquivo. O que fazer é configurado em g7_acao.",
    gateG8Title: "G8 · A conversa foi encerrada pelo cliente?",
    gateG8Description:
        "Quando o cliente se despede, manda a despedida sem chamar o modelo de texto.",
    preservedHandoffDescription: "Motivos de transferência para humano.",
    preservedHandoffWhere: "Roteamento",
    preservedFarewellDescription:
        "Texto enviado quando o G8 encerra a conversa. Sem a chave, vale o texto padrão.",
    preservedSafeReplyDescription:
        "Texto enviado quando o G6 reprova a resposta duas vezes.",
    preservedG7ActionDescription:
        "O que fazer com injeção ou abuso: registrar (padrão), responder_padrao ou transferir.",
    preservedG7ReplyDescription:
        "Texto usado quando g7_acao é responder_padrao. Sem a chave, vale o texto padrão.",
    validationProviderRequired: "informe o provedor",
    validationModelRequired: "informe o modelo",
    validationTemperatureLocked:
        "a tela não volta ao padrão do servidor: digite um valor entre 0 e 2",
    validationTemperatureRange: "use um número entre 0 e 2",
    validationMaxOutputLocked:
        "a tela não volta ao padrão do servidor: digite um número de tokens",
    validationMaxOutputPositive: "use um número inteiro maior que zero",
    validationMaxContextMin: "use um número de {min} para cima",
    validationGroupingWindowRange: "use um número de 0 a {max} ms",
    validationMaxToolRoundsRange: "use um número de 0 a {max}",
    validationMaxToolCallsRange: "use um número de 0 a {max}",
    validationRagTopKRange: "use um número de {min} a {max}",
    validationRagMinScoreRange: "use um número de {min} a {max}",
    gateStatusDisabledBySwitch: "desligado pelo interruptor do Jev deste agente",
    gateStatusInherit: "{state} (herda o padrão de fábrica)",
    gateStatusActiveInAgent: "ligado neste agente",
    gateStatusInactiveInAgent: "desligado neste agente",
    gateStateOnLower: "ligado",
    gateStateOffLower: "desligado",
    loadAgentFallbackError: "falha ao carregar o agente",
    nothingToSave: "Nada para gravar: o modelo e o Jev já estão como estão salvos.",
    savedNotice: "Modelo e Jev gravados (versão {version}). Vale a partir do próximo turno.",
    conflictMessage:
        "Outra pessoa salvou este agente antes de você{versionSuffix}. O que você digitou foi mantido: revise e salve de novo.",
    conflictVersionSuffix: " (agora na versão {version})",
    apiErrorWithReason: "{message}: {reason} (código {code})",
    apiErrorWithoutReason: "{message} (código {code})",
    unexpectedSaveError: "falha inesperada ao salvar o modelo e o Jev",
    discardedNotice: "Alterações descartadas: voltou ao que está salvo.",
    loadErrorTitle: "Não consegui carregar o modelo e o Jev do agente",
    loadErrorDescription:
        "A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor.",
    retry: "Tentar de novo",
    backToAgents: "Voltar para agentes",
    headerDescription: "modelo, busca na base e portões de decisão do Jev",
    bannerOutdatedTitle: "Versão desatualizada",
    bannerSaveErrorTitle: "Não foi possível salvar",
    sectionModelTitle: "Modelo de linguagem",
    sectionModelDescription:
        "Quem escreve a resposta e com que limites. Temperatura e máximo de tokens de saída mostram “herda o padrão do servidor” quando o agente ainda não tem valor; depois de definidos, a tela não consegue voltar a esse padrão.",
    fieldProviderLabel: "Provedor",
    fieldProviderHint: "Na V1 o provedor é antigravity2api.",
    fieldModelLabel: "Modelo",
    fieldModelHint: "Nome do modelo que escreve a resposta.",
    fieldTemperatureLabel: "Temperatura",
    fieldTemperatureHint: "De {min} a {max}. Mais alto, respostas mais variadas.",
    fieldMaxOutputLabel: "Máximo de tokens de saída",
    fieldMaxOutputHint: "Tamanho máximo de cada resposta, em tokens.",
    fieldMaxContextLabel: "Máximo de tokens de contexto",
    fieldMaxContextHint:
        "Quanto da conversa e das instruções o turno monta para o modelo. Mínimo {min}.",
    placeholderInheritsDefault: "herda o padrão do servidor",
    sectionServiceTitle: "Atendimento e ferramentas",
    sectionServiceDescription:
        "Como o turno junta as mensagens do cliente e quantas ferramentas pode usar.",
    fieldGroupingWindowLabel: "Janela de agrupamento (ms)",
    fieldGroupingWindowHint:
        "De 0 a {max} ms. Mensagens que chegam dentro dessa janela viram uma resposta só.",
    fieldMaxToolRoundsLabel: "Rodadas de ferramenta por turno",
    fieldMaxToolRoundsHint:
        "Quantas vezes o modelo pode chamar ferramentas numa mesma resposta. De 0 a {max}.",
    fieldMaxToolCallsLabel: "Ferramentas por mensagem",
    fieldMaxToolCallsHint:
        "Limite de chamadas de ferramenta para cada mensagem do cliente. De 0 a {max}.",
    sectionRagTitle: "Busca na base (RAG)",
    sectionRagDescription:
        "Quantos trechos da base de conhecimento entram na resposta e o mínimo de relevância para aceitar um trecho.",
    fieldRagTopKLabel: "Trechos recuperados (top-k)",
    fieldRagTopKHint: "De {min} a {max}.",
    fieldRagMinScoreLabel: "Corte mínimo de relevância",
    fieldRagMinScoreHint:
        "De {min} a {max}. Zero aceita qualquer trecho do top-k, mesmo sem relação com a pergunta.",
    sectionJevTitle: "Jev: portões de decisão",
    sectionJevDescription:
        "Cada gate decide uma coisa antes da resposta. “Herdar” usa o padrão de fábrica do código (G3, G5 e G8 ligados; os demais desligados). A configuração do servidor pode ter mudado isso, e esta tela não consegue ler essa configuração.",
    switchDisableJevLabel: "Desligar o Jev neste agente",
    switchDisableJevDescription:
        "Quando ligado, nenhum gate roda neste agente, mesmo com os gates abaixo ligados.",
    gateHintToday: "{explanation} Hoje: {status}.",
    gateOptionInherit: "Herdar (padrão de fábrica: {defaultState})",
    gateOptionOn: "Ligado",
    gateOptionOff: "Desligado",
    sectionOtherKeysTitle: "Outras chaves do Jev (não editadas aqui)",
    sectionOtherKeysDescription:
        "Estas chaves o motor lê, mas esta aba não as altera. Ao salvar, elas voltam exatamente como estão.",
    keyDefined: "definida",
    keyUndefined: "não definida",
    keyEditIn: " Edite em {where}.",
    statusUnsaved: "Alterações não salvas",
    statusSaved: "Salvo (versão {version})",
    discardChanges: "Descartar alterações",
    saveButton: "Salvar modelo e Jev",
} as const satisfies Record<string, string>;
