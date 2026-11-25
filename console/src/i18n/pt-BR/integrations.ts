export const integrations = {
    statusActive: "Ativa",
    statusInactive: "Inativa",
    statusArchived: "Arquivada",
    connectionConnected: "Conectado",
    connectionDisconnected: "Desconectado",
    connectionAwaitingScan: "Aguardando leitura do QR",
    connectionConnecting: "Conectando",
    whatsappTipNotConfigured:
        "Esta integração ainda não tem uma instância da Evolution declarada na configuração do servidor. Enquanto isso, não há como parear nem consultar o estado dela.",
    whatsappTipEvolutionUnavailable:
        "O servidor não conseguiu falar com o WhatsApp: a integração está sem a chave da Evolution ou o serviço está fora do ar.",
    whatsappTipNotWhatsapp:
        "Só a integração do tipo whatsapp tem pareamento, estado e desconexão.",
    whatsappTipEvolutionRefused:
        "A Evolution recusou a operação. O motivo dela está na mensagem acima; tente de novo em alguns segundos.",
    whatsappTipEvolutionNoQr:
        "A Evolution respondeu sem QR nem código. Peça outro QR em alguns segundos.",
    whatsappTipVersionConflict:
        "Outra pessoa alterou esta integração antes de você. A tela releu a versão atual; repita a operação.",
    whatsappTipPreconditionRequired:
        "A tela precisa da versão atual da integração para esta operação.",
    whatsappUnexpectedFailure: "falha inesperada ao falar com o WhatsApp",
    validationKindRequired: "informe o tipo da integração (por exemplo, whatsapp)",
    validationKindMaxLength: "o tipo aceita até {max} caracteres",
    validationStatusRequired: "escolha a situação",
    schemaValidationSecretRequired: "informe o segredo",
    schemaValidationFieldRequired: "campo obrigatório",
    schemaValidationEnum: "use um destes valores: {options}",
    schemaValidationNumber: "informe um número",
    schemaValidationInteger: "informe um número inteiro",
    schemaValidationMin: "o mínimo é {min}",
    schemaValidationMax: "o máximo é {max}",
    schemaEmptyManifest: "Esta integração não tem configuração nem segredo no manifesto.",
    schemaSecretRemovingHint: "o segredo gravado será removido ao salvar",
    schemaSecretSavedHint: "já gravado: deixe em branco para manter, digite para trocar",
    schemaSecretUnreadableHint:
        "há segredo gravado, mas a chave de criptografia atual não consegue lê-lo: digite um valor novo para voltar a valer",
    schemaSelectChoose: "Escolha…",
    schemaSecretSavedPlaceholder: "•••••• (gravado; digite para trocar)",
    schemaSecretUnreadablePlaceholder: "•••••• (gravado; ilegível com a chave atual)",
    schemaSecretDefaultPlaceholder: "•••••• (guardado; digite para trocar)",
    schemaSecretRemoveCheckbox: "Remover este segredo ao salvar",
    schemaSecretRequiredNote: "Segredo obrigatório: não pode ser removido, só trocado.",
    schemaSecretUnreadableNote:
        "A tela não sabe qual segredo está gravado porque a chave de criptografia do servidor mudou. Regravar resolve.",
    manifestTipUnavailable:
        "O plugin deste tipo está declarado na configuração do servidor, mas não respondeu. Sem o formulário dele não há como gravar configuração — confira o plugin e tente de novo.",
    manifestTipNetwork:
        "Não foi possível falar com a instância para ler o formulário do plugin.",
    manifestTipNotFound: "Este tipo não declara formulário no servidor.",
    manifestInvalidError: "o servidor respondeu sem o esquema de configuração deste tipo",
    manifestReadError: "falha ao ler o formulário do plugin",
    manifestLoadingSaveError: "Aguarde: ainda estou lendo o formulário deste tipo.",
    modalConflict:
        "Outra pessoa alterou esta integração antes de você{versionSuffix}. O que você preencheu foi mantido: revise e salve de novo.",
    modalConflictVersionSuffix: " (agora na versão {version})",
    modalUnexpectedSaveError: "falha inesperada ao salvar a integração",
    modalTitleEdit: "Editar integração",
    modalTitleNew: "Nova integração",
    modalDescriptionEdit:
        "O tipo não muda depois de criado. O salvamento usa a versão lida; configuração e segredo entram pelo formulário do plugin.",
    modalDescriptionNew:
        "Escolha o tipo: o formulário de configuração é o que o plugin daquele tipo declara no servidor.",
    modalCloseLabel: "Fechar formulário de integração",
    modalCancel: "Cancelar",
    modalSaveChanges: "Salvar alterações",
    modalCreate: "Criar integração",
    bannerOutdatedTitle: "Versão desatualizada",
    bannerSaveErrorTitle: "Não foi possível salvar",
    fieldKindLabel: "Tipo da integração",
    fieldKindHint:
        "Identifica o canal no servidor. Exemplos: {whatsapp}, eco. Até {max} caracteres; não muda depois de criado.",
    connectionStateLabel: "Estado de conexão (informado pelo canal)",
    manifestLoadingText: "Lendo o formulário deste tipo…",
    pluginConfigHeader: "Configuração do plugin “{name}” (versão {version})",
    noManifestTitle: "Este tipo não tem formulário",
    noManifestDescription:
        "O servidor não declara manifesto para “{kind}”: a integração é criada só com o tipo e a situação, e nenhuma configuração é gravada.",
    manifestErrorTitle: "Não consegui ler o formulário do plugin",
    manifestErrorCode: " (código: {code})",
    fieldStatusLabel: "Situação",
    fieldStatusHint:
        "Inativa e arquivada param o atendimento por este canal; arquivada é o estado final.",
    unreadableSecretsNote:
        "Há segredo gravado nesta integração, mas a chave de criptografia atual do servidor não consegue lê-lo: a tela não sabe quais campos estão preenchidos. Digitar o valor de novo resolve.",
    secretsBlankNote:
        "Segredo em branco mantém o que já está gravado; digitar troca o valor. O valor de um segredo nunca volta para esta tela depois de salvo.",
    whatsappConflictText:
        "Outra pessoa alterou esta integração antes de você (agora na versão {version}). O estado foi relido: revise e repita a operação.",
    whatsappConflictTip: "Nada foi perdido: a integração continua como está no servidor.",
    whatsappConflictRereadFailed:
        "Outra pessoa alterou esta integração antes de você, e a releitura falhou. Recarregue a lista e repita.",
    whatsappPhoneInvalid: "use só dígitos, com DDI e DDD (ex.: 5511999998888)",
    whatsappQrWithCodeNotice: "QR gerado. Leia com o celular ou use o código de pareamento.",
    whatsappQrNotice: "QR gerado. Leia com o WhatsApp do celular.",
    whatsappAlreadyConnectedNotice: "Este aparelho já está conectado; não há QR para ler.",
    whatsappLiveStateNotice: "Estado ao vivo: {state}.",
    whatsappPollingStoppedNotice:
        "Paramos de consultar o estado automaticamente depois de 3 minutos. Use “Verificar estado” ou “Gerar outro QR”.",
    whatsappConnectedNotice: "Aparelho conectado.",
    whatsappDisconnectedNotice:
        "Aparelho desconectado. A integração continua cadastrada; para voltar, gere um novo QR.",
    whatsappSecretSaved: "segredo gravado — nunca exibido",
    whatsappNoSecret: "sem segredo gravado",
    whatsappInstanceLabel: "instância: {instance}",
    whatsappPhoneLabel: "Número do WhatsApp (opcional)",
    whatsappPhoneHint:
        "Só dígitos, com DDI e DDD. Em branco, o pareamento é pelo QR. Com o número, o WhatsApp também devolve o código de 8 caracteres.",
    whatsappRegenerateQr: "Gerar outro QR",
    whatsappGenerateQr: "Conectar / gerar QR",
    whatsappCheckState: "Verificar estado",
    whatsappDisconnectDevice: "Desconectar aparelho",
    whatsappStopPolling: "Parar consulta automática",
    whatsappOperationErrorTitle: "Não consegui completar a operação",
    whatsappErrorCode: "código: {code}",
    whatsappQrAlt: "QR de pareamento do WhatsApp",
    whatsappQrHeading: "Leia este QR com o WhatsApp do celular",
    whatsappQrSteps: "No celular: WhatsApp → Aparelhos conectados → Conectar aparelho.",
    whatsappQrRound: "rodada {round} do QR",
    whatsappPairingCode: "Código de pareamento:",
    whatsappQrRotationHint:
        "O QR muda sozinho a cada ~20 s e não tem prazo informado pelo WhatsApp. Se demorar, clique em “Gerar outro QR”.",
    whatsappConnectedBannerTitle: "Aparelho conectado",
    whatsappConnectedBannerBody:
        "O aparelho está conectado a esta integração; não há QR para ler. Para trocar de aparelho, desconecte e gere um novo QR.",
    whatsappNoQrHint:
        "Nenhum QR na tela. Use “Conectar / gerar QR” para parear o celular; o estado acima é o que o servidor tem gravado até você consultar o estado ao vivo.",
    whatsappPollingHint: "Consultando o estado a cada {seconds} s por até {minutes} min.",
    whatsappDisconnectModalTitle: "Desconectar o WhatsApp?",
    whatsappDisconnectModalDescription:
        "O aparelho para de receber e enviar mensagens por esta integração.",
    whatsappDisconnectModalClose: "Fechar confirmação de desconexão",
    whatsappDisconnectCancel: "Cancelar",
    whatsappDisconnectConfirm: "Desconectar",
    whatsappDisconnectBodyPrefix: "A integração ",
    whatsappDisconnectBodyStrong: "continua cadastrada",
    whatsappDisconnectBodySuffix:
        "; o que sai é o pareamento do aparelho. Para reconectar, é só gerar um novo QR e ler com o celular.",
    whatsappDisconnectIntegrationId: "integração {id}",
    loadIntegrationsFallbackError: "falha ao carregar as integrações",
    loadAgentFallbackError: "falha ao carregar o agente",
    loadAgentErrorTitle: "Não consegui carregar as integrações do agente",
    loadAgentErrorDescription:
        "A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor.",
    retry: "Tentar de novo",
    backToAgents: "Voltar para agentes",
    actionEdit: "Editar",
    actionDelete: "Excluir",
    loadListErrorTitle: "Não consegui carregar as integrações",
    loadListErrorDescription: "A lista de integrações não respondeu. Tente de novo.",
    emptyTitle: "Nenhuma integração neste agente",
    emptyDescription:
        "Uma integração liga o agente a um canal, como o WhatsApp. Crie a primeira para o agente atender por ela.",
    emptyAction: "Criar primeira integração",
    tableAriaLabel: "Integrações do agente",
    colType: "Tipo",
    colStatus: "Situação",
    colConnection: "Conexão",
    colSecret: "Segredo",
    colConfig: "Configuração",
    colVersion: "Versão",
    colActions: "Ações",
    rowMenuLabel: "Ações de {kind} ({id})",
    secretSavedCell: "gravado (não exibido)",
    secretNoneCell: "sem segredo",
    configEmptyCell: "sem configuração",
    configOneField: "1 campo gravado",
    configManyFields: "{count} campos gravados",
    footerOne: "1 integração",
    footerMany: "{count} integrações",
    headerDescription: "canais e plugins ligados a este agente",
    refresh: "Atualizar",
    newIntegration: "Nova integração",
    pluginFormBannerTitle: "Configuração pelo formulário do plugin",
    pluginFormBannerBody:
        "O formulário de cada integração vem do manifesto que o próprio plugin declara no servidor — nenhuma integração tem formulário escrito à mão aqui. Tipo sem plugin declarado (como o {whatsapp}, que usa a Evolution) é criado só com tipo e situação. O pareamento, o estado ao vivo e a desconexão do WhatsApp funcionam no painel abaixo.",
    sectionIntegrationsTitle: "Integrações do agente",
    sectionIntegrationsHelp:
        "O tipo identifica o canal. A situação controla se o agente atende por ele; alterar usa a versão lida (duas pessoas não sobrescrevem uma à outra).",
    sectionWhatsappTitle: "WhatsApp: pareamento e conexão",
    sectionWhatsappHelp:
        "Gere o QR, acompanhe o estado ao vivo e desconecte o aparelho. Sem contagem regressiva: o WhatsApp não informa validade do QR.",
    whatsappEmpty:
        "Nenhuma integração do tipo {whatsapp}. Crie uma abaixo com o tipo {whatsapp} para parear um aparelho aqui.",
    sectionSecretsTitle: "Segredos",
    sectionSecretsHelp:
        "Toda credencial de integração fica guardada cifrada no servidor e nunca volta na resposta, no log ou nesta tela.",
    secretsFooterNote:
        "A coluna “Segredo” só diz se existe um segredo gravado, e o formulário do plugin mostra no máximo quais campos já estão preenchidos. O valor não é legível por esta API — é o desenho, não uma limitação da tela.",
    savedNotice: "Integração {kind} gravada (versão {version}).",
    createdNotice: "Integração {kind} criada.",
    deletedNotice: "Integração {kind} excluída.",
    deleteModalTitle: "Excluir integração?",
    deleteModalDescription:
        "O agente deixa de atender por “{kind}” e a configuração desta integração some. Não há como desfazer; para voltar, é preciso criar outra.",
    deleteModalResourceLabel: "tipo da integração",
    deleteModalConfirmLabel: "Excluir integração",
    deleteModalConsequence:
        "Se houver mais de uma integração do mesmo tipo, confira o identificador {id} na lista: a confirmação é pelo tipo.",
} as const satisfies Record<string, string>;
