
import { request } from "./client";
import type { components } from "./schema";

export type MatrixState = components["schemas"]["EstadoMatrixJson"];
export type Espaco = components["schemas"]["EspacoJson"];
export type DisabledSpace = components["schemas"]["EspacoDesligadoJson"];
export type SpaceInput = components["schemas"]["LigarEspaco"];

function base(org: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/matrix`;
}

export async function readMatrixState(org: string): Promise<MatrixState> {
    return request<MatrixState>(base(org));
}

export function stateVersion(estado: MatrixState): number {
    return estado.updated_at;
}

export async function updateMatrixModule(
    org: string,
    enabled: boolean,
    versao: number,
): Promise<MatrixState> {
    const body: components["schemas"]["LigarDesligar"] = { enabled: enabled };
    return request<MatrixState>(base(org), {
        method: "PUT",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(body),
    });
}

export async function listMatrixSpaces(org: string): Promise<Espaco[]> {
    const response = await request<components["schemas"]["EspacosJson"]>(`${base(org)}/spaces`);
    return response.items;
}

export async function linkMatrixSpace(org: string, input: SpaceInput): Promise<Espaco> {
    const body: SpaceInput = { department_id: input.department_id };
    if (input.space && input.space.trim()) {
        body.space = input.space.trim();
    }
    return request<Espaco>(`${base(org)}/spaces`, {
        method: "PUT",
        body: JSON.stringify(body),
    });
}

export async function unlinkMatrixSpace(
    org: string,
    departamento: string,
): Promise<DisabledSpace> {
    return request<DisabledSpace>(`${base(org)}/spaces/${encodeURIComponent(departamento)}`, {
        method: "DELETE",
    });
}
