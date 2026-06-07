import { createLazyFileRoute } from '@tanstack/react-router'
import { TraceUploadPage } from '../pages/TraceUploadPage'

export const Route = createLazyFileRoute('/upload')({
  component: TraceUploadPage,
})
