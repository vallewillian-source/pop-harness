import { login as ptBRLogin } from "../pt-BR/login";

export const login: Record<keyof typeof ptBRLogin, string> = {
    title: "POP Harness Console",
    subtitle: "Token-based administrative access. Internal use.",
    expiredTitle: "Session ended",
    expiredBody: "This session's token is no longer accepted by the instance.",
    offlineTitle: "Offline",
    invalidToken:
        "Invalid or revoked administrative token. Check with the operations team or generate another one with `poph token`.",
    unreachableInstance:
        "Could not reach the instance at {origin}. Verify that the `poph serve` process is running.",
    unexpectedError: "Unexpected sign-in failure. Please try again.",
    tokenLabel: "Administrative token",
    tokenHint: "Generated with `poph token` on the instance, or set in [admin].tokens_globais.",
    tokenPlaceholder: "pophg_… or poph_…",
    showToken: "Show token",
    hideToken: "Hide token",
    instanceLabel: "Instance",
    submit: "Sign in to Console",
    sessionNotice:
        "The token stays only in this tab's memory and browser session storage: closing the tab ends access. Nothing is stored permanently.",
    scopeNotice:
        "Scope is defined by the token: global sees all organizations, organization sees only its own.",
    emptyToken: "enter the administrative token",
};
