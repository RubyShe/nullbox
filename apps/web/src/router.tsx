import { createBrowserRouter } from "react-router";
import { AppShell } from "./components/AppShell";
import { CustomerTicketPage } from "./routes/customer/CustomerTicketPage";
import { NewTicketPage } from "./routes/customer/NewTicketPage";
import { AuditLogPage } from "./routes/admin/AuditLogPage";
import { NotFoundPage } from "./routes/NotFoundPage";
import { CheckoutCancelPage, CheckoutSuccessPage } from "./routes/shop/CheckoutResultPage";
import { UpgradePage } from "./routes/shop/UpgradePage";
import { InboxPage } from "./routes/worker/InboxPage";
import { WorkerTicketPage } from "./routes/worker/WorkerTicketPage";

/**
 * All routes, in one place. For Tauri (no server to rewrite deep links),
 * swap `createBrowserRouter` for `createHashRouter`: that's the only change.
 */
export const router = createBrowserRouter([
  {
    element: <AppShell />,
    children: [
      // customer portal
      { path: "/", element: <NewTicketPage /> },
      { path: "/t/:id", element: <CustomerTicketPage /> },
      // worker inbox
      { path: "/inbox", element: <InboxPage /> },
      { path: "/inbox/:id", element: <WorkerTicketPage /> },
      // admin
      { path: "/admin/audit", element: <AuditLogPage /> },
      // payments
      { path: "/upgrade", element: <UpgradePage /> },
      { path: "/checkout/success", element: <CheckoutSuccessPage /> },
      { path: "/checkout/cancel", element: <CheckoutCancelPage /> },
      { path: "*", element: <NotFoundPage /> },
    ],
  },
]);
