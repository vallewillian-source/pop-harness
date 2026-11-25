export const files = {
    refresh: "Atualizar",
    upload: "Enviar arquivo",
    uploadFirst: "Enviar primeiro arquivo",
    uploadedNotice: 'Arquivo "{name}" enviado à biblioteca.',
    removedNotice: 'Arquivo "{name}" removido da biblioteca.',
    columnFile: "Arquivo",
    columnType: "Tipo",
    columnSize: "Tamanho",
    columnActions: "Ações",
    deleteFileLabel: "Excluir arquivo {name}",
    apiErrorWithCode: "{message} (código {code})",
    agentLoadFailed: "falha ao carregar o agente",
    libraryLoadFailed: "falha ao carregar a biblioteca",
    loadErrorTitle: "Não consegui carregar os arquivos do agente",
    loadErrorDescription:
        "A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor.",
    libraryErrorTitle: "Não consegui carregar a biblioteca",
    libraryErrorDescription: "A lista de arquivos não respondeu. Tente de novo.",
    retry: "Tentar de novo",
    backToAgents: "Voltar para agentes",
    emptyTitle: "Nenhum arquivo na biblioteca",
    emptyDescription:
        "Sem arquivo aqui, o agente não tem o que enviar ao cliente. Envie PDFs, planilhas, apresentações ou imagens de até 16 MB.",
    tableLabel: "Biblioteca de arquivos do agente",
    countOne: "1 arquivo",
    countMany: "{total} arquivos",
    headerDescription: "arquivos que o agente pode enviar ao cliente",
    sectionTitle: "Biblioteca de arquivos",
    sectionDescription:
        "O agente só envia ao cliente o que está aqui. Ele escolhe pelo nome e pela descrição; nunca escolhe um caminho no disco. Limite: 16 MB por arquivo.",
    deleteTitle: "Excluir arquivo?",
    deleteDescription:
        "O agente deixa de poder enviar “{name}” ao cliente. Não há como desfazer; para voltar, é preciso enviar o arquivo de novo.",
    deleteResourceLabel: "nome do arquivo",
    deleteConfirm: "Excluir arquivo",
    fileEmpty: "o arquivo está vazio",
    fileTooLarge:
        "o arquivo passa do limite de 16 MB por arquivo em {excess}. Reduza ou comprima o arquivo antes de enviar.",
    chooseFile: "escolha o arquivo a enviar",
    nameTooLong: "o nome aceita até {limit} caracteres",
    descriptionTooLong: "a descrição aceita até {limit} caracteres",
    uploadUnexpected: "falha inesperada ao enviar o arquivo",
    uploadTitle: "Enviar arquivo ao agente",
    uploadDescription:
        "O agente só envia ao cliente o que estiver nesta biblioteca. Ele escolhe pelo nome e pela descrição.",
    uploadClose: "Fechar envio de arquivo",
    uploadFailedTitle: "Não foi possível enviar",
    fileLabel: "Arquivo",
    fileHint:
        "PDF, Word (.docx), Excel (.xlsx ou .xls), PowerPoint (.pptx) ou imagem (JPEG, PNG, WebP, GIF). Até 16 MB.",
    chosenFile: "Escolhido: {name} · {size} (dentro do limite de 16 MB)",
    nameLabel: "Nome mostrado ao cliente",
    nameHint:
        "Opcional. Se ficar vazio, usa o nome do arquivo. Até {limit} caracteres.",
    descriptionLabel: "Descrição para o agente",
    descriptionHint:
        "Opcional, mas é o que o agente lê para decidir quando enviar. Até {limit} caracteres.",
} as const satisfies Record<string, string>;
