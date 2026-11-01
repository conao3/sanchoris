import { Children, isValidElement, type ReactNode } from "react"
import ReactMarkdown, { type Components } from "react-markdown"
import remarkGfm from "remark-gfm"
import { CodeBlock } from "./CodeBlock.tsx"

const LANGUAGE = /language-(\S+)/

function fence(children: ReactNode): { code: string; language: string } | undefined {
  const [child] = Children.toArray(children)
  if (!isValidElement<{ className?: string; children?: ReactNode }>(child)) {
    return undefined
  }
  const code = child.props.children
  if (typeof code !== "string") {
    return undefined
  }
  return {
    code: code.replace(/\n$/, ""),
    language: LANGUAGE.exec(child.props.className ?? "")?.[1] ?? "text",
  }
}

const components: Components = {
  p: ({ children }) => <p className="my-2 first:mt-0 last:mb-0">{children}</p>,
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noreferrer" className="text-run underline">
      {children}
    </a>
  ),
  ul: ({ children }) => <ul className="my-2 list-disc pl-5">{children}</ul>,
  ol: ({ children }) => <ol className="my-2 list-decimal pl-5">{children}</ol>,
  li: ({ children }) => <li className="my-0.5">{children}</li>,
  h1: ({ children }) => <h1 className="mt-4 mb-1.5 text-base font-semibold">{children}</h1>,
  h2: ({ children }) => <h2 className="mt-4 mb-1.5 text-base font-semibold">{children}</h2>,
  h3: ({ children }) => <h3 className="mt-3 mb-1 font-semibold">{children}</h3>,
  h4: ({ children }) => <h4 className="mt-3 mb-1 font-semibold">{children}</h4>,
  blockquote: ({ children }) => (
    <blockquote className="my-2 border-l-2 border-line2 pl-3 text-muted">{children}</blockquote>
  ),
  hr: () => <hr className="my-3 border-line" />,
  code: ({ children }) => (
    <code className="rounded-kbd bg-hover px-1 py-px font-mono text-[0.85em]">{children}</code>
  ),
  pre: ({ children }) => {
    const block = fence(children)
    return block ? (
      <CodeBlock code={block.code} language={block.language} />
    ) : (
      <pre className="my-2 overflow-x-auto rounded-ui bg-sub p-3 font-mono text-xs">{children}</pre>
    )
  },
  table: ({ children }) => (
    <div className="my-2 overflow-x-auto">
      <table className="border-collapse text-sm">{children}</table>
    </div>
  ),
  th: ({ children }) => (
    <th className="border border-line bg-sub px-2 py-1 text-left font-semibold">{children}</th>
  ),
  td: ({ children }) => <td className="border border-line px-2 py-1">{children}</td>,
}

export function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={className}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  )
}
