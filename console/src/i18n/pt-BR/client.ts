export const client = {
    networkError: "não foi possível falar com a instância do POP Harness ({reason})",
    httpError: "a API respondeu {status}",
    healthError: "a instância respondeu {status}",
} as const satisfies Record<string, string>;
