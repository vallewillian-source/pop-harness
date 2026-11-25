import { modelJev as ptBR_modelJev } from "../pt-BR/modelJev";

export const modelJev: Record<keyof typeof ptBR_modelJev, string> = {
    gateG1Title: "G1 · Hand off to human now?",
    gateG1Description:
        "Decides whether the conversation goes to a human agent. Includes health-risk guidance, which depends on company policy.",
    gateG2Title: "G2 · Wait for more messages or reply now?",
    gateG2Description:
        "Adjusts the wait before replying when the customer sends messages in sequence.",
    gateG3Title: "G3 · Needs knowledge base, product, or neither?",
    gateG3Description:
        "For greetings and thanks, skips querying the knowledge base and the product catalog.",
    gateG4Title: "G4 · Which department receives the handoff?",
    gateG4Description:
        "Chooses one of the organization's active departments on handoff. Without a valid choice, uses the agent's default department.",
    gateG5Title: "G5 · Which product does the customer want?",
    gateG5Description:
        "Picks the product among catalog candidates when more than one is possible. Enabling it runs product search on every turn.",
    gateG6Title: "G6 · Can the reply be sent?",
    gateG6Description:
        "Reviews the reply before sending. If rejected, the model rewrites it once; if rejected again, the agent's safe reply is sent.",
    gateG7Title: "G7 · Is it prompt injection or abuse?",
    gateG7Description:
        "Detects attempts to give orders to the assistant or abuse in the message or file text. What to do is configured in g7_acao.",
    gateG8Title: "G8 · Did the customer end the conversation?",
    gateG8Description:
        "When the customer says goodbye, sends the farewell without calling the text model.",
    preservedHandoffDescription: "Reasons for handoff to a human.",
    preservedHandoffWhere: "Routing",
    preservedFarewellDescription:
        "Text sent when G8 ends the conversation. Without this key, the default text applies.",
    preservedSafeReplyDescription:
        "Text sent when G6 rejects the reply twice.",
    preservedG7ActionDescription:
        "What to do on injection or abuse: registrar (default), responder_padrao, or transferir.",
    preservedG7ReplyDescription:
        "Text used when g7_acao is responder_padrao. Without this key, the default text applies.",
    validationProviderRequired: "enter the provider",
    validationModelRequired: "enter the model",
    validationTemperatureLocked:
        "the screen cannot revert to the server default: enter a value between 0 and 2",
    validationTemperatureRange: "use a number between 0 and 2",
    validationMaxOutputLocked:
        "the screen cannot revert to the server default: enter a token count",
    validationMaxOutputPositive: "use an integer greater than zero",
    validationMaxContextMin: "use a number of {min} or higher",
    validationGroupingWindowRange: "use a number from 0 to {max} ms",
    validationMaxToolRoundsRange: "use a number from 0 to {max}",
    validationMaxToolCallsRange: "use a number from 0 to {max}",
    validationRagTopKRange: "use a number from {min} to {max}",
    validationRagMinScoreRange: "use a number from {min} to {max}",
    gateStatusDisabledBySwitch: "turned off by this agent's Jev switch",
    gateStatusInherit: "{state} (inherits factory default)",
    gateStatusActiveInAgent: "enabled on this agent",
    gateStatusInactiveInAgent: "disabled on this agent",
    gateStateOnLower: "enabled",
    gateStateOffLower: "disabled",
    loadAgentFallbackError: "failed to load the agent",
    nothingToSave: "Nothing to save: model and Jev already match what is saved.",
    savedNotice: "Model and Jev saved (version {version}). Takes effect on the next turn.",
    conflictMessage:
        "Someone else saved this agent before you{versionSuffix}. What you typed was kept: review and save again.",
    conflictVersionSuffix: " (now at version {version})",
    apiErrorWithReason: "{message}: {reason} (code {code})",
    apiErrorWithoutReason: "{message} (code {code})",
    unexpectedSaveError: "unexpected failure while saving model and Jev",
    discardedNotice: "Changes discarded: reverted to what is saved.",
    loadErrorTitle: "Could not load the agent's model and Jev",
    loadErrorDescription:
        "The instance did not respond when reading the agent. Try again; if it persists, check instance health.",
    retry: "Try again",
    backToAgents: "Back to agents",
    headerDescription: "model, knowledge base search, and Jev decision gates",
    bannerOutdatedTitle: "Outdated version",
    bannerSaveErrorTitle: "Could not save",
    sectionModelTitle: "Language model",
    sectionModelDescription:
        "Who writes the reply and under what limits. Temperature and max output tokens show “inherits server default” when the agent has no value yet; once set, the screen cannot revert to that default.",
    fieldProviderLabel: "Provider",
    fieldProviderHint: "In V1, the provider is antigravity2api.",
    fieldModelLabel: "Model",
    fieldModelHint: "Name of the model that writes the reply.",
    fieldTemperatureLabel: "Temperature",
    fieldTemperatureHint: "From {min} to {max}. Higher values produce more varied replies.",
    fieldMaxOutputLabel: "Max output tokens",
    fieldMaxOutputHint: "Maximum size of each reply, in tokens.",
    fieldMaxContextLabel: "Max context tokens",
    fieldMaxContextHint:
        "How much of the conversation and instructions the turn sends to the model. Minimum {min}.",
    placeholderInheritsDefault: "inherits server default",
    sectionServiceTitle: "Service and tools",
    sectionServiceDescription:
        "How the turn groups customer messages and how many tools it can call.",
    fieldGroupingWindowLabel: "Grouping window (ms)",
    fieldGroupingWindowHint:
        "From 0 to {max} ms. Messages arriving within this window become a single reply.",
    fieldMaxToolRoundsLabel: "Tool rounds per turn",
    fieldMaxToolRoundsHint:
        "How many times the model can call tools in a single reply. From 0 to {max}.",
    fieldMaxToolCallsLabel: "Tool calls per message",
    fieldMaxToolCallsHint:
        "Limit of tool calls for each customer message. From 0 to {max}.",
    sectionRagTitle: "Knowledge base search (RAG)",
    sectionRagDescription:
        "How many knowledge base chunks enter the reply and the minimum relevance to accept a chunk.",
    fieldRagTopKLabel: "Retrieved chunks (top-k)",
    fieldRagTopKHint: "From {min} to {max}.",
    fieldRagMinScoreLabel: "Minimum relevance cutoff",
    fieldRagMinScoreHint:
        "From {min} to {max}. Zero accepts any top-k chunk, even if unrelated to the question.",
    sectionJevTitle: "Jev: decision gates",
    sectionJevDescription:
        "Each gate decides one thing before the reply. “Inherit” uses the code's factory default (G3, G5, and G8 enabled; the rest disabled). Server configuration may have changed this, and this screen cannot read that configuration.",
    switchDisableJevLabel: "Turn off Jev on this agent",
    switchDisableJevDescription:
        "When enabled, no gate runs on this agent, even if individual gates below are enabled.",
    gateHintToday: "{explanation} Current: {status}.",
    gateOptionInherit: "Inherit (factory default: {defaultState})",
    gateOptionOn: "Enabled",
    gateOptionOff: "Disabled",
    sectionOtherKeysTitle: "Other Jev keys (not edited here)",
    sectionOtherKeysDescription:
        "The engine reads these keys, but this tab does not edit them. When saving, they are preserved as-is.",
    keyDefined: "defined",
    keyUndefined: "not defined",
    keyEditIn: " Edit in {where}.",
    statusUnsaved: "Unsaved changes",
    statusSaved: "Saved (version {version})",
    discardChanges: "Discard changes",
    saveButton: "Save model and Jev",
};
