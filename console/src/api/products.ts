
import { request } from "./client";
import { listStructure, type Category } from "./structure";
import { listAgents, getAgent, type Agente } from "./agents";

export type ProductStatus = "active" | "inactive" | "archived";

export type Product = {
    id: string;
    org_id: string;
    category_id: string;
    name: string;
    slug: string;
    aliases: string[];
    description: string;
    price_text: string;
    price_cents: number;
    unit: string;
    includes: string;
    excludes: string;
    conditions: string;
    tags: string[];
    status: ProductStatus;
    version: number;
    created_at: number;
    updated_at: number;
    index_warning?: string | null;
};

export type ProductInput = {
    category_id: string;
    name: string;
    slug?: string;
    aliases: string[];
    description: string;
    price_text: string;
    price_cents: number;
    unit: string;
    includes: string;
    excludes: string;
    conditions: string;
    tags: string[];
};

export type ProductChange = Partial<Omit<ProductInput, "slug">> & {
    slug?: string;
    status?: ProductStatus;
};

export type ProductDeleteResult = {
    deleted: boolean;
    index_warning?: string | null;
};

function base(org: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/products`;
}

export async function listProducts(org: string): Promise<Product[]> {
    const items: Product[] = [];
    let cursor: string | null = null;
    do {
        const suffix = cursor ? `&cursor=${encodeURIComponent(cursor)}` : "";
        const response: { items: Product[]; next_cursor: string | null } = await request(
            `${base(org)}?all=true${suffix}`,
        );
        items.push(...response.items);
        cursor = response.next_cursor ?? null;
    } while (cursor);
    return items;
}

export async function getProduct(org: string, id: string): Promise<Product> {
    return request<Product>(`${base(org)}/${encodeURIComponent(id)}`);
}

export async function createProduct(org: string, input: ProductInput): Promise<Product> {
    const body: Record<string, unknown> = { ...input };
    if (!input.slug || !input.slug.trim()) {
        delete body["slug"];
    }
    return request<Product>(base(org), {
        method: "POST",
        body: JSON.stringify(body),
    });
}

export async function updateProduct(
    org: string,
    id: string,
    versao: number,
    change: ProductChange,
): Promise<Product> {
    return request<Product>(`${base(org)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(change),
    });
}

export async function deleteProduct(
    org: string,
    id: string,
    forcar = false,
): Promise<ProductDeleteResult> {
    const suffix = forcar ? "?force=true" : "";
    return request<ProductDeleteResult>(`${base(org)}/${encodeURIComponent(id)}${suffix}`, {
        method: "DELETE",
    });
}

export type CartCount = { "itens_de_carrinho": number; "carrinhos": number };

export function refusalCount(details: unknown): CartCount | null {
    if (!details || typeof details !== "object") return null;
    const { "itens_de_carrinho": cartItems, "carrinhos": carts } = details as Record<string, unknown>;
    if (typeof cartItems !== "number" || typeof carts !== "number") return null;
    return { "itens_de_carrinho": cartItems, "carrinhos": carts };
}

export async function listCategories(org: string): Promise<Category[]> {
    return (await listStructure("categoria", org)) as Category[];
}

export async function loadAgentsWithCategories(org: string): Promise<Agente[]> {
    const agents = await listAgents(org);
    return Promise.all(agents.map((agente) => getAgent(org, agente.id)));
}

export function agentsSeeingProduct(agents: Agente[], product: Product): Agente[] {
    return agents.filter((agente) => {
        const categories = agente.product_category_ids ?? [];
        return categories.length === 0 || categories.includes(product.category_id);
    });
}
