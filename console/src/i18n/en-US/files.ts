import { files as ptBR_files } from "../pt-BR/files";

export const files: Record<keyof typeof ptBR_files, string> = {
    refresh: "Refresh",
    upload: "Upload file",
    uploadFirst: "Upload first file",
    uploadedNotice: 'File "{name}" uploaded to the library.',
    removedNotice: 'File "{name}" removed from the library.',
    columnFile: "File",
    columnType: "Type",
    columnSize: "Size",
    columnActions: "Actions",
    deleteFileLabel: "Delete file {name}",
    apiErrorWithCode: "{message} (code {code})",
    agentLoadFailed: "failed to load the agent",
    libraryLoadFailed: "failed to load the library",
    loadErrorTitle: "Could not load the agent files",
    loadErrorDescription:
        "The instance did not answer the agent read. Try again; if it persists, check the server health.",
    libraryErrorTitle: "Could not load the library",
    libraryErrorDescription: "The file list did not answer. Try again.",
    retry: "Try again",
    backToAgents: "Back to agents",
    emptyTitle: "No file in the library",
    emptyDescription:
        "Without a file here, the agent has nothing to send to the customer. Upload PDFs, spreadsheets, presentations or images up to 16 MB.",
    tableLabel: "Agent file library",
    countOne: "1 file",
    countMany: "{total} files",
    headerDescription: "files the agent can send to the customer",
    sectionTitle: "File library",
    sectionDescription:
        "The agent only sends the customer what is here. It chooses by name and description; it never picks a path on disk. Limit: 16 MB per file.",
    deleteTitle: "Delete file?",
    deleteDescription:
        "The agent can no longer send “{name}” to the customer. There is no undo; to bring it back, upload the file again.",
    deleteResourceLabel: "file name",
    deleteConfirm: "Delete file",
    fileEmpty: "the file is empty",
    fileTooLarge:
        "the file goes over the 16 MB per-file limit by {excess}. Shrink or compress the file before uploading.",
    chooseFile: "choose the file to upload",
    nameTooLong: "the name accepts up to {limit} characters",
    descriptionTooLong: "the description accepts up to {limit} characters",
    uploadUnexpected: "unexpected failure while uploading the file",
    uploadTitle: "Upload file to the agent",
    uploadDescription:
        "The agent only sends the customer what is in this library. It chooses by name and description.",
    uploadClose: "Close file upload",
    uploadFailedTitle: "Could not upload",
    fileLabel: "File",
    fileHint:
        "PDF, Word (.docx), Excel (.xlsx or .xls), PowerPoint (.pptx) or image (JPEG, PNG, WebP, GIF). Up to 16 MB.",
    chosenFile: "Chosen: {name} · {size} (within the 16 MB limit)",
    nameLabel: "Name shown to the customer",
    nameHint:
        "Optional. If left empty, the file name is used. Up to {limit} characters.",
    descriptionLabel: "Description for the agent",
    descriptionHint:
        "Optional, but it is what the agent reads to decide when to send. Up to {limit} characters.",
};
