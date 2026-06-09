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

function BreakButton() {
  return (
    <a
      href="http://139.196.146.229/games"
      target="_blank"
      rel="noopener noreferrer"
      className="group fixed right-6 bottom-6 z-50 flex items-center gap-2 rounded-full border border-stone-200/50 bg-white/60 px-5 py-2.5 text-[13px] text-stone-400 backdrop-blur-xl shadow-sm transition-all duration-500 ease-out hover:border-stone-300/60 hover:bg-white/80 hover:text-stone-600 hover:shadow-md active:scale-[0.97]"
    >
      <span className="text-sm transition-transform duration-500 ease-out group-hover:rotate-12">☕</span>
      <span className="tracking-[0.1em]">累了？休息一下</span>
    </a>
  )
}

function RootLayout() {
  return (
    <>
      <Outlet />
      <BreakButton />
    </>
  )
}

export const Route = rootRoute
