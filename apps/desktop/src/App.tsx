import { BrowserRouter } from "react-router";
import { AppShell } from "@/components/shell/AppShell";

export default function App() {
  return (
    <BrowserRouter>
      <AppShell />
    </BrowserRouter>
  );
}
