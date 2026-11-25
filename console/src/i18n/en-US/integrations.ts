import { integrations as ptBR_integrations } from "../pt-BR/integrations";

export const integrations: Record<keyof typeof ptBR_integrations, string> = {
    statusActive: "Active",
    statusInactive: "Inactive",
    statusArchived: "Archived",
    connectionConnected: "Connected",
    connectionDisconnected: "Disconnected",
    connectionAwaitingScan: "Awaiting QR scan",
    connectionConnecting: "Connecting",
    whatsappTipNotConfigured:
        "This integration does not have an Evolution instance declared in the server configuration yet. Meanwhile, it cannot pair or check its state.",
    whatsappTipEvolutionUnavailable:
        "The server could not reach WhatsApp: the integration is missing the Evolution key or the service is down.",
    whatsappTipNotWhatsapp:
        "Only whatsapp integrations support pairing, live state, and disconnection.",
    whatsappTipEvolutionRefused:
        "Evolution refused the operation. Its reason is in the message above; try again in a few seconds.",
    whatsappTipEvolutionNoQr:
        "Evolution replied without a QR or code. Request another QR in a few seconds.",
    whatsappTipVersionConflict:
        "Someone else modified this integration before you. The screen re-read the current version; repeat the operation.",
    whatsappTipPreconditionRequired:
        "The screen needs the current integration version for this operation.",
    whatsappUnexpectedFailure: "unexpected failure while talking to WhatsApp",
    validationKindRequired: "enter the integration type (for example, whatsapp)",
    validationKindMaxLength: "the type accepts up to {max} characters",
    validationStatusRequired: "choose the status",
    schemaValidationSecretRequired: "enter the secret",
    schemaValidationFieldRequired: "required field",
    schemaValidationEnum: "use one of these values: {options}",
    schemaValidationNumber: "enter a number",
    schemaValidationInteger: "enter an integer",
    schemaValidationMin: "minimum is {min}",
    schemaValidationMax: "maximum is {max}",
    schemaEmptyManifest: "This integration has no configuration or secrets in the manifest.",
    schemaSecretRemovingHint: "the saved secret will be removed when saving",
    schemaSecretSavedHint: "already saved: leave blank to keep, type to replace",
    schemaSecretUnreadableHint:
        "there is a saved secret, but the current encryption key cannot read it: enter a new value to make it valid again",
    schemaSelectChoose: "Choose…",
    schemaSecretSavedPlaceholder: "•••••• (saved; type to replace)",
    schemaSecretUnreadablePlaceholder: "•••••• (saved; unreadable with current key)",
    schemaSecretDefaultPlaceholder: "•••••• (stored; type to replace)",
    schemaSecretRemoveCheckbox: "Remove this secret on save",
    schemaSecretRequiredNote: "Required secret: cannot be removed, only replaced.",
    schemaSecretUnreadableNote:
        "The screen cannot tell which secret is saved because the server encryption key changed. Re-saving fixes it.",
    manifestTipUnavailable:
        "This type's plugin is declared in the server configuration, but did not respond. Without its form, configuration cannot be saved — check the plugin and try again.",
    manifestTipNetwork:
        "Could not reach the instance to read the plugin form.",
    manifestTipNotFound: "This type does not declare a form on the server.",
    manifestInvalidError: "the server replied without the configuration schema for this type",
    manifestReadError: "failed to read the plugin form",
    manifestLoadingSaveError: "Please wait: still reading the form for this type.",
    modalConflict:
        "Someone else modified this integration before you{versionSuffix}. What you filled in was kept: review and save again.",
    modalConflictVersionSuffix: " (now at version {version})",
    modalUnexpectedSaveError: "unexpected failure while saving the integration",
    modalTitleEdit: "Edit integration",
    modalTitleNew: "New integration",
    modalDescriptionEdit:
        "The type cannot be changed after creation. Saving uses the version that was read; configuration and secrets come from the plugin form.",
    modalDescriptionNew:
        "Choose the type: the configuration form is declared on the server by that type's plugin.",
    modalCloseLabel: "Close integration form",
    modalCancel: "Cancel",
    modalSaveChanges: "Save changes",
    modalCreate: "Create integration",
    bannerOutdatedTitle: "Outdated version",
    bannerSaveErrorTitle: "Could not save",
    fieldKindLabel: "Integration type",
    fieldKindHint:
        "Identifies the channel on the server. Examples: {whatsapp}, eco. Up to {max} characters; cannot change after creation.",
    connectionStateLabel: "Connection state (reported by the channel)",
    manifestLoadingText: "Reading the form for this type…",
    pluginConfigHeader: "Plugin “{name}” configuration (version {version})",
    noManifestTitle: "This type has no form",
    noManifestDescription:
        "The server does not declare a manifest for “{kind}”: the integration is created with type and status only, and no configuration is saved.",
    manifestErrorTitle: "Could not read the plugin form",
    manifestErrorCode: " (code: {code})",
    fieldStatusLabel: "Status",
    fieldStatusHint:
        "Inactive and archived stop service on this channel; archived is the final state.",
    unreadableSecretsNote:
        "There is a secret saved on this integration, but the server's current encryption key cannot read it: the screen cannot tell which fields are filled. Typing the value again fixes it.",
    secretsBlankNote:
        "Leaving a secret blank keeps what is already saved; typing replaces the value. A secret's value never returns to this screen after saving.",
    whatsappConflictText:
        "Someone else modified this integration before you (now at version {version}). The state was re-read: review and repeat the operation.",
    whatsappConflictTip: "Nothing was lost: the integration remains as it is on the server.",
    whatsappConflictRereadFailed:
        "Someone else modified this integration before you, and re-reading failed. Reload the list and try again.",
    whatsappPhoneInvalid: "use digits only, with country and area code (e.g. 5511999998888)",
    whatsappQrWithCodeNotice: "QR generated. Scan with your phone or use the pairing code.",
    whatsappQrNotice: "QR generated. Scan with WhatsApp on your phone.",
    whatsappAlreadyConnectedNotice: "This device is already connected; there is no QR to scan.",
    whatsappLiveStateNotice: "Live state: {state}.",
    whatsappPollingStoppedNotice:
        "Stopped polling the state automatically after 3 minutes. Use “Check state” or “Generate another QR”.",
    whatsappConnectedNotice: "Device connected.",
    whatsappDisconnectedNotice:
        "Device disconnected. The integration remains registered; to reconnect, generate a new QR.",
    whatsappSecretSaved: "secret saved — never shown",
    whatsappNoSecret: "no secret saved",
    whatsappInstanceLabel: "instance: {instance}",
    whatsappPhoneLabel: "WhatsApp number (optional)",
    whatsappPhoneHint:
        "Digits only, with country and area code. When blank, pairing uses the QR. With a number, WhatsApp also returns the 8-character code.",
    whatsappRegenerateQr: "Generate another QR",
    whatsappGenerateQr: "Connect / generate QR",
    whatsappCheckState: "Check state",
    whatsappDisconnectDevice: "Disconnect device",
    whatsappStopPolling: "Stop automatic polling",
    whatsappOperationErrorTitle: "Could not complete the operation",
    whatsappErrorCode: "code: {code}",
    whatsappQrAlt: "WhatsApp pairing QR",
    whatsappQrHeading: "Scan this QR with WhatsApp on your phone",
    whatsappQrSteps: "On your phone: WhatsApp → Linked devices → Link a device.",
    whatsappQrRound: "QR round {round}",
    whatsappPairingCode: "Pairing code:",
    whatsappQrRotationHint:
        "The QR rotates automatically every ~20 s and has no expiration reported by WhatsApp. If it takes too long, click “Generate another QR”.",
    whatsappConnectedBannerTitle: "Device connected",
    whatsappConnectedBannerBody:
        "The device is connected to this integration; there is no QR to scan. To switch devices, disconnect and generate a new QR.",
    whatsappNoQrHint:
        "No QR on screen. Use “Connect / generate QR” to pair your phone; the state above is what the server has saved until you check live state.",
    whatsappPollingHint: "Checking state every {seconds} s for up to {minutes} min.",
    whatsappDisconnectModalTitle: "Disconnect WhatsApp?",
    whatsappDisconnectModalDescription:
        "The device stops receiving and sending messages through this integration.",
    whatsappDisconnectModalClose: "Close disconnect confirmation",
    whatsappDisconnectCancel: "Cancel",
    whatsappDisconnectConfirm: "Disconnect",
    whatsappDisconnectBodyPrefix: "The integration ",
    whatsappDisconnectBodyStrong: "remains registered",
    whatsappDisconnectBodySuffix:
        "; only the device pairing is removed. To reconnect, just generate a new QR and scan it with your phone.",
    whatsappDisconnectIntegrationId: "integration {id}",
    loadIntegrationsFallbackError: "failed to load integrations",
    loadAgentFallbackError: "failed to load the agent",
    loadAgentErrorTitle: "Could not load the agent's integrations",
    loadAgentErrorDescription:
        "The instance did not respond when reading the agent. Try again; if it persists, check instance health.",
    retry: "Try again",
    backToAgents: "Back to agents",
    actionEdit: "Edit",
    actionDelete: "Delete",
    loadListErrorTitle: "Could not load integrations",
    loadListErrorDescription: "The integrations list did not respond. Try again.",
    emptyTitle: "No integrations on this agent",
    emptyDescription:
        "An integration connects the agent to a channel, such as WhatsApp. Create the first one so the agent can handle messages through it.",
    emptyAction: "Create first integration",
    tableAriaLabel: "Agent integrations",
    colType: "Type",
    colStatus: "Status",
    colConnection: "Connection",
    colSecret: "Secret",
    colConfig: "Configuration",
    colVersion: "Version",
    colActions: "Actions",
    rowMenuLabel: "Actions for {kind} ({id})",
    secretSavedCell: "saved (not shown)",
    secretNoneCell: "no secret",
    configEmptyCell: "no configuration",
    configOneField: "1 field saved",
    configManyFields: "{count} fields saved",
    footerOne: "1 integration",
    footerMany: "{count} integrations",
    headerDescription: "channels and plugins connected to this agent",
    refresh: "Refresh",
    newIntegration: "New integration",
    pluginFormBannerTitle: "Configuration via plugin form",
    pluginFormBannerBody:
        "Each integration's form comes from the manifest declared by the plugin on the server — no integration has a hand-coded form here. A type without a declared plugin (such as {whatsapp}, which uses Evolution) is created with type and status only. WhatsApp pairing, live state, and disconnection work in the panel below.",
    sectionIntegrationsTitle: "Agent integrations",
    sectionIntegrationsHelp:
        "The type identifies the channel. The status controls whether the agent handles messages through it; editing uses the version that was read (two people do not overwrite each other).",
    sectionWhatsappTitle: "WhatsApp: pairing and connection",
    sectionWhatsappHelp:
        "Generate the QR, follow the live state, and disconnect the device. No countdown: WhatsApp does not report QR expiration.",
    whatsappEmpty:
        "No {whatsapp} integration yet. Create one below with type {whatsapp} to pair a device here.",
    sectionSecretsTitle: "Secrets",
    sectionSecretsHelp:
        "Every integration credential is stored encrypted on the server and never comes back in responses, logs, or this screen.",
    secretsFooterNote:
        "The “Secret” column only indicates whether a secret is saved, and the plugin form at most shows which fields are already filled. The value cannot be read through this API — that is by design, not a screen limitation.",
    savedNotice: "Integration {kind} saved (version {version}).",
    createdNotice: "Integration {kind} created.",
    deletedNotice: "Integration {kind} deleted.",
    deleteModalTitle: "Delete integration?",
    deleteModalDescription:
        "The agent stops handling messages through “{kind}” and this integration's configuration is removed. This cannot be undone; to restore it, you must create another.",
    deleteModalResourceLabel: "integration type",
    deleteModalConfirmLabel: "Delete integration",
    deleteModalConsequence:
        "If there is more than one integration of the same type, check the identifier {id} in the list: confirmation is by type.",
};
