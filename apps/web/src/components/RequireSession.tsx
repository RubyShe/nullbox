import { Navigate, Outlet, useLocation } from "react-router";
import { ApiError } from "../api/client";
import { useSession } from "../api/queries";
import { ErrorMessage, Loading } from "./Feedback";

export function RequireSession() {
  const session = useSession();
  const location = useLocation();
  if (session.isPending) return <Loading />;
  if (session.error instanceof ApiError && session.error.status === 401) return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  if (session.error) return <ErrorMessage error={session.error} />;
  return <Outlet />;
}
