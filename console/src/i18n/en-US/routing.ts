import { routing as ptBRRouting } from "../pt-BR/routing";

export const routing: Record<keyof typeof ptBRRouting, string> = {
    validationHumanDepartment: "choose the human destination department to turn on the transfer",
    validationTooManyReasons: "use at most {limit} reasons (there are {count})",
    validationReasonTooLong: "the reason accepts up to {limit} characters",
    departmentInactive: "{name} (inactive)",
    departmentNotFound: "department not found in the organization",
    loadAgentFailed: "failed to load the agent",
    loadDepartmentsFailed: "failed to load the departments",
    nothingToSave: "Nothing to save: the routing is already as saved.",
    savedVersion: "Routing saved (version {version}).",
    conflictMessage:
        "Someone else saved this agent before you. What you typed was kept: review and save again.",
    conflictMessageVersion:
        "Someone else saved this agent before you (now at version {version}). What you typed was kept: review and save again.",
    errorWithDetails: "{message}: {reason} (code {code})",
    errorWithCode: "{message} (code {code})",
    unexpectedSaveError: "unexpected failure while saving the routing",
    loadErrorTitle: "Could not load the agent routing",
    loadErrorDescription:
        "The instance did not respond to reading the agent. Try again; if it persists, check the server health.",
    retry: "Try again",
    headerDescription: "where the conversation goes and when it moves to a human",
    conflictTitle: "Outdated version",
    saveErrorTitle: "Could not save",
    departmentsErrorTitle: "Could not load the departments",
    sectionDepartments: "Agent departments",
    sectionDepartmentsDescription:
        "Departments belong to the organization (created in Structure). Here you choose which ones the agent uses.",
    emptyDepartmentsTitle: "No department in the organization",
    emptyDepartmentsDescription:
        "Create the departments in Structure before choosing the default and the transfer department.",
    goToStructure: "Go to Structure",
    defaultDepartmentLabel: "Default department",
    defaultDepartmentHint:
        "Where the conversation of this agent starts. It can only be changed; the API does not accept leaving it without a department.",
    chooseDepartment: "Choose a department",
    handoffLabel: "Transfer to a human agent",
    handoffDescription:
        "Turns on the transfer: when the engine decides the case needs a human, the conversation goes to the destination department below.",
    humanDepartmentLabel: "Human destination department",
    humanDepartmentHint:
        "Where the conversation goes when there is a transfer. Required with the transfer turned on.",
    sectionReasons: "Transfer reasons",
    sectionReasonsDescription:
        "Situations in which the conversation must move to a human, in plain language (e.g. “customer asks for a second copy”). Up to {limit} reasons, with up to {maxLength} characters each. Empty reasons are not saved.",
    addReason: "Add reason",
    noReasons:
        "No reason registered. Without reasons, the agent only transfers in the risk cases the engine already handles.",
    reasonLabel: "Reason {number}",
    removeReasonLabel: "Remove reason {number}",
    unsavedChanges: "Unsaved changes",
    savedStatus: "Saved (version {version})",
    discardChanges: "Discard changes",
    saveRouting: "Save routing",
    discarded: "Changes discarded: back to what is saved.",
};
