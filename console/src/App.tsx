
import { useEffect } from "react";
import { Navigate, Outlet, Route, Routes, useLocation, useParams } from "react-router-dom";
import { GuardChangesProvider } from "./components/GuardChanges";
import { Shell } from "./components/Shell";
import { resolveDefaultOrgPath, useSession, writeLastOrgId } from "./session/Session";
import { AgentKnowledge } from "./screens/AgentKnowledge";
import { AgentFiles } from "./screens/AgentFiles";
import { AgentChatTest } from "./screens/AgentChatTest";
import { AgentInstructions } from "./screens/AgentInstructions";
import { AgentModelJev } from "./screens/AgentModelJev";
import { AgentIntegrations } from "./screens/AgentIntegrations";
import { AgentRouting } from "./screens/AgentRouting";
import { Agents } from "./screens/Agents";
import { Login } from "./screens/Login";
import { Structure } from "./screens/Structure";
import { OrgPanel } from "./screens/OrgPanel";
import { Orgs } from "./screens/Orgs";
import { Products } from "./screens/Products";
import { Operation } from "./screens/Operation";
import { Matrix } from "./screens/Matrix";

function RememberLastOrg() {
    const { "sessao": session } = useSession();
    const { pathname } = useLocation();
    useEffect(() => {
        const [root, orgId] = pathname.split("/").filter(Boolean);
        if (root !== "orgs" || !orgId) return;
        if (session?.orgs.some((org) => org.id === orgId)) writeLastOrgId(orgId);
    }, [pathname, session]);
    return null;
}

function Protegido() {
    const { "sessao": session } = useSession();
    if (!session) {
        return <Navigate to="/login" replace />;
    }
    return (
        <GuardChangesProvider>
            <RememberLastOrg />
            <Shell>
                <Outlet />
            </Shell>
        </GuardChangesProvider>
    );
}

function SessionHome() {
    const { "sessao": session } = useSession();
    return <Navigate to={resolveDefaultOrgPath(session?.orgs ?? [])} replace />;
}

function LoginOrSignIn() {
    const { "sessao": session } = useSession();
    return session ? <Navigate to={resolveDefaultOrgPath(session.orgs)} replace /> : <Login />;
}

function AgentWithoutTab() {
    const { orgId = "", agentId = "" } = useParams();
    return <Navigate to={`/orgs/${orgId}/agents/${agentId}/instructions`} replace />;
}

export function App() {
    return (
        <Routes>
            <Route path="/login" element={<LoginOrSignIn />} />
            <Route element={<Protegido />}>
                <Route path="/" element={<SessionHome />} />
                <Route path="/orgs" element={<Orgs />} />
                <Route path="/orgs/:orgId" element={<OrgPanel />} />
                <Route path="/orgs/:orgId/structure" element={<Structure />} />
                <Route path="/orgs/:orgId/products" element={<Products />} />
                <Route path="/orgs/:orgId/operacao" element={<Operation />} />
                <Route path="/orgs/:orgId/matrix" element={<Matrix />} />
                <Route path="/orgs/:orgId/agents" element={<Agents />} />
                <Route path="/orgs/:orgId/agents/:agentId" element={<AgentWithoutTab />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/instructions"
                    element={<AgentInstructions />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/knowledge-base"
                    element={<AgentKnowledge />}
                />
                <Route path="/orgs/:orgId/agents/:agentId/files" element={<AgentFiles />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/routing"
                    element={<AgentRouting />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/integrations"
                    element={<AgentIntegrations />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/model-jev"
                    element={<AgentModelJev />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/test-chat"
                    element={<AgentChatTest />}
                />
                <Route path="*" element={<SessionHome />} />
            </Route>
        </Routes>
    );
}
