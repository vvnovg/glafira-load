import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Layout } from "./components/Layout";
import { useAppMenu } from "./lib/useAppMenu";
import { useLanguageSync } from "./lib/useLanguageSync";

function LanguageSync() {
  useLanguageSync();
  return null;
}

/** Меню Help на macOS (на других системах ничего не делает). */
function AppMenu() {
  useAppMenu();
  return null;
}

// Без автоповтора: повторить можно кнопкой в баннере ошибки.
// Без перезапроса при возвращении сети и фокуса окна: вкладки остаются смонтированными,
// и повторные запросы выполняли бы работу, которую пользователь не просил.
const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false, refetchOnReconnect: false } },
});

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <LanguageSync />
      <AppMenu />
      <Layout />
    </QueryClientProvider>
  );
}
