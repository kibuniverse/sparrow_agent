import { createRootRoute, Link, Outlet } from '@tanstack/react-router'

const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: () => (
    <div className="flex min-h-dvh flex-col items-center justify-center gap-4">
      <h1 className="text-2xl font-semibold text-slate-950">404</h1>
      <p className="text-sm text-slate-600">页面不存在</p>
      <Link
        to="/"
        className="text-sm text-blue-600 hover:text-blue-800 hover:underline"
      >
        返回首页
      </Link>
    </div>
  ),
})

function RootLayout() {
  return <Outlet />
}

export const Route = rootRoute
