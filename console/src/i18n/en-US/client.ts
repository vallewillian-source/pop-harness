import { client as ptBRClient } from "../pt-BR/client";

export const client: Record<keyof typeof ptBRClient, string> = {
    networkError: "could not reach the POP Harness instance ({reason})",
    httpError: "the API responded {status}",
    healthError: "the instance responded {status}",
};
