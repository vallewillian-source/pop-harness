/**
 * Menu lateral do Console (PRD do re-layout §5.2).
 *
 * Desde a RL-03: seletor de organização, três grupos (Tenants e recursos, Runtime e motor, Sistema)
 * lidos de `navegacao.ts`, contador em Organizações e em Agentes (esse último vem de fora, pela
 * casca), marcador âmbar no item ativo e rodapé com a versão da instância e o botão de atalhos.
 * Largura de 232 px, como no desenho aprovado (HR-61). A casca põe este menu abaixo da barra
 * superior.
 */

import { Fragment, type ReactNode } from "react";
import { NavLink, useLocation } from "react-router-dom";
import { SofaButton, SofaRailItem, SofaSelect, SofaSidebarRail } from "@sofacoding/sofa-ui";
import { useSessao } from "../sessao/Sessao";
import { useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import {
    GRUPOS_DE_NAVEGACAO,
    ITENS_DE_NAVEGACAO,
    type ContextoDeNavegacao,
    type ItemDeNavegacao,
} from "./navegacao";
import { useSaude } from "./saude";
import "../estilos/navegacao.css";

/**
 * Motivo que aparece ao passar o mouse num item ativo que está sem contexto na URL. Não é texto
 * visível na linha: vai no `title` do wrapper.
 */
function motivoDoBloqueio(item: ItemDeNavegacao, contexto: ContextoDeNavegacao): string | undefined {
    if (item.exigeOrg && !contexto.orgId) return "Requer uma organização aberta";
    if (item.exigeAgente && !contexto.agentId) return "Requer um agente aberto";
    return undefined;
}

/**
 * Um item do menu. Sem rota vira botão desabilitado; com rota, vira `NavLink` para a rota
 * resolvida. "em breve" aparece só nos itens de estado `emBreve`. Um item ativo sem contexto
 * (organização ou agente ausente na URL) fica desabilitado sem etiqueta visível: o motivo vai no
 * `title` de um wrapper, porque o `SofaRailItem` não repassa `title` nem `aria-*`.
 */
export function ItenDeNavegacao({
    item,
    contexto,
}: {
    item: ItemDeNavegacao;
    contexto: ContextoDeNavegacao;
}) {
    const icone: ReactNode = <item.icone aria-hidden="true" />;
    const contagem = item.contador?.(contexto) ?? null;
    const aside: ReactNode =
        contagem === null ? undefined : <span className="poph-rail__contador">{contagem}</span>;
    const rota = item.rota(contexto);
    if (item.estado === "emBreve" || rota === null) {
        const emBreve = item.estado === "emBreve";
        const botao = (
            <SofaRailItem
                className="poph-rail__item"
                label={item.rotulo}
                media={icone}
                aside={aside}
                description={emBreve ? "em breve" : undefined}
                disabled
            />
        );
        const motivo = emBreve ? undefined : motivoDoBloqueio(item, contexto);
        if (!motivo) return botao;
        return (
            <div className="poph-rail__bloqueado" title={motivo}>
                {botao}
            </div>
        );
    }
    return (
        <NavLink
            to={rota}
            end={item.exato === true}
            style={{ color: "inherit", textDecoration: "none" }}
        >
            {({ isActive }) => (
                <SofaRailItem
                    className="poph-rail__item"
                    label={item.rotulo}
                    media={icone}
                    aside={aside}
                    active={isActive}
                />
            )}
        </NavLink>
    );
}

export function Rail({
    aoAbrirAtalhos,
    totalDeAgentes = null,
}: {
    aoAbrirAtalhos: () => void;
    /** Agentes da organização aberta (contador do item Agentes); `null` = sem contador. */
    totalDeAgentes?: number | null;
}) {
    const { sessao } = useSessao();
    const guarda = useGuardaDeAlteracoes();
    const { saude: dados } = useSaude();
    const { pathname } = useLocation();
    // Organização aberta na URL (`/orgs/:id/...`): habilita os itens que dependem dela.
    const partesDaUrl = pathname.split("/").filter(Boolean);
    const orgAberta = partesDaUrl[0] === "orgs" ? (partesDaUrl[1] ?? "") : "";
    const agenteAberto = orgAberta && partesDaUrl[2] === "agents" ? (partesDaUrl[3] ?? "") : "";

    const orgs = sessao?.orgs ?? [];
    // Com token de org o seletor é informativo: mostra a própria org e não deixa trocar.
    const selecionada = sessao?.escopo === "org" ? (orgs[0]?.id ?? "") : orgAberta;
    const contexto: ContextoDeNavegacao = {
        orgId: orgAberta || null,
        agentId: agenteAberto || null,
        totalDeOrgs: sessao ? orgs.length : null,
        totalDeAgentes,
    };

    return (
        <SofaSidebarRail
            width={232}
            footer={
                <div className="poph-rail__base">
                    <span className="poph-rail__versao">
                        POP Harness v{dados?.version ?? "—"} · Rust
                    </span>
                    <SofaButton variant="subtle" size="sm" onClick={aoAbrirAtalhos}>
                        ? atalhos
                    </SofaButton>
                </div>
            }
        >
            <div className="poph-rail__seletor">
                <label className="poph-rail__rotulo" htmlFor="poph-org-ativa">
                    Organização
                </label>
                <SofaSelect
                    id="poph-org-ativa"
                    value={selecionada}
                    disabled={orgs.length === 0 || sessao?.escopo === "org"}
                    onChange={(evento) => {
                        const valor = evento.target.value;
                        if (valor) guarda.tentarNavegar(`/orgs/${valor}`);
                    }}
                >
                    {sessao?.escopo !== "org" && (
                        <option value="">
                            {orgs.length === 0 ? "Nenhuma organização" : "Todas"}
                        </option>
                    )}
                    {orgs.map((org) => (
                        <option key={org.id} value={org.id}>
                            {org.name}
                        </option>
                    ))}
                </SofaSelect>
            </div>

            {GRUPOS_DE_NAVEGACAO.map((grupo) => (
                <Fragment key={grupo}>
                    <div className="sofa-sidebar-rail__section-title">{grupo}</div>
                    {ITENS_DE_NAVEGACAO.filter((item) => item.grupo === grupo).map((item) => (
                        <ItenDeNavegacao key={item.rotulo} item={item} contexto={contexto} />
                    ))}
                </Fragment>
            ))}
        </SofaSidebarRail>
    );
}
