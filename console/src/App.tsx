/**
 * Rotas do Console. A ordem é de propósito: `/login` fica fora da casca, tudo o mais exige sessão.
 */

import { useEffect } from "react";
import { Navigate, Outlet, Route, Routes, useLocation, useParams } from "react-router-dom";
import { GuardaAlteracoesProvider } from "./componentes/GuardaAlteracoes";
import { Shell } from "./componentes/Shell";
import { resolveDefaultOrgPath, useSessao, writeLastOrgId } from "./sessao/Sessao";
import { AgenteConhecimento } from "./telas/AgenteConhecimento";
import { AgenteArquivos } from "./telas/AgenteArquivos";
import { AgenteChatTeste } from "./telas/AgenteChatTeste";
import { AgenteInstrucoes } from "./telas/AgenteInstrucoes";
import { AgenteModeloJev } from "./telas/AgenteModeloJev";
import { AgenteIntegracoes } from "./telas/AgenteIntegracoes";
import { AgenteRoteamento } from "./telas/AgenteRoteamento";
import { Agentes } from "./telas/Agentes";
import { Login } from "./telas/Login";
import { Estrutura } from "./telas/Estrutura";
import { OrgPainel } from "./telas/OrgPainel";
import { Orgs } from "./telas/Orgs";
import { Produtos } from "./telas/Produtos";
import { Operacao } from "./telas/Operacao";
import { Matrix } from "./telas/Matrix";

/**
 * Guarda a organização aberta na URL como a última do navegador, quando ela é uma das que o token
 * enxerga. Vive dentro da casca e não desenha nada.
 */
function RememberLastOrg() {
    const { sessao } = useSessao();
    const { pathname } = useLocation();
    useEffect(() => {
        const [root, orgId] = pathname.split("/").filter(Boolean);
        if (root !== "orgs" || !orgId) return;
        if (sessao?.orgs.some((org) => org.id === orgId)) writeLastOrgId(orgId);
    }, [pathname, sessao]);
    return null;
}

/** Casca autenticada: sem sessão, volta para o login. A guarda de alterações vive dentro dela. */
function Protegido() {
    const { sessao } = useSessao();
    if (!sessao) {
        return <Navigate to="/login" replace />;
    }
    return (
        <GuardaAlteracoesProvider>
            <RememberLastOrg />
            <Shell>
                <Outlet />
            </Shell>
        </GuardaAlteracoesProvider>
    );
}

/** Porta de entrada do Console: a última organização aberta, a primeira da sessão ou a lista. */
function SessionHome() {
    const { sessao } = useSessao();
    return <Navigate to={resolveDefaultOrgPath(sessao?.orgs ?? [])} replace />;
}

/** `/login` já autenticado não fica na tela de login. */
function LoginOuEntrar() {
    const { sessao } = useSessao();
    return sessao ? <Navigate to={resolveDefaultOrgPath(sessao.orgs)} replace /> : <Login />;
}

/** `/orgs/:orgId/agents/:agentId` abre a aba de Instruções (a primeira que existe). */
function AgenteSemAba() {
    const { orgId = "", agentId = "" } = useParams();
    return <Navigate to={`/orgs/${orgId}/agents/${agentId}/instructions`} replace />;
}

export function App() {
    return (
        <Routes>
            <Route path="/login" element={<LoginOuEntrar />} />
            <Route element={<Protegido />}>
                <Route path="/" element={<SessionHome />} />
                <Route path="/orgs" element={<Orgs />} />
                <Route path="/orgs/:orgId" element={<OrgPainel />} />
                <Route path="/orgs/:orgId/structure" element={<Estrutura />} />
                <Route path="/orgs/:orgId/products" element={<Produtos />} />
                <Route path="/orgs/:orgId/operacao" element={<Operacao />} />
                <Route path="/orgs/:orgId/matrix" element={<Matrix />} />
                <Route path="/orgs/:orgId/agents" element={<Agentes />} />
                <Route path="/orgs/:orgId/agents/:agentId" element={<AgenteSemAba />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/instructions"
                    element={<AgenteInstrucoes />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/knowledge-base"
                    element={<AgenteConhecimento />}
                />
                <Route path="/orgs/:orgId/agents/:agentId/files" element={<AgenteArquivos />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/routing"
                    element={<AgenteRoteamento />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/integrations"
                    element={<AgenteIntegracoes />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/model-jev"
                    element={<AgenteModeloJev />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/test-chat"
                    element={<AgenteChatTeste />}
                />
                <Route path="*" element={<SessionHome />} />
            </Route>
        </Routes>
    );
}
