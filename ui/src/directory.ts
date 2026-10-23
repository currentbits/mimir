/** Hosted MCP servers offered with one click under Settings → Tool servers. Each URL is on the company's own domain
 *  (most are also in the official MCP registry under that domain), answers MCP over Streamable HTTP and either needs no
 *  sign-in or offers MCP's sign-in with app registration, so Add just works. `header`: no such sign-in; the person
 *  pastes a token into this header. `app`: no app registration there; the person registers one (how: this text) and
 *  pastes its client id and secret. Checked 2026-10-19; Box, HubSpot, Cal.com and Calendly 2026-10-23. */
export type Listed = { name: string; category: string; desc: string; url: string; header?: string; app?: string }

// ponytail: a hand-kept list, not the registry: registry search returns lookalikes (third-party "kraken", "ahrefs")
// that would get the person's sign-in. Registry search can come later, limited to publishers that own the URL's domain.
export const DIRECTORY: Listed[] = [
  { category: 'Finance', name: 'Stripe', desc: 'Customers, payments, invoices and subscriptions', url: 'https://mcp.stripe.com' },
  { category: 'Finance', name: 'PayPal', desc: 'Invoices, orders, refunds and disputes', url: 'https://mcp.paypal.com/mcp' },
  { category: 'Finance', name: 'Square', desc: 'Payments, orders, catalog and customers', url: 'https://mcp.squareup.com/mcp' },
  { category: 'Finance', name: 'Ramp', desc: 'Spend, transactions, bills and approvals', url: 'https://mcp.ramp.com/mcp' },
  { category: 'Finance', name: 'Mercury', desc: 'Bank accounts, balances and transactions', url: 'https://mcp.mercury.com/mcp' },

  { category: 'Research', name: 'Parallel', desc: 'Web search and page extraction built for agents', url: 'https://search-mcp.parallel.ai/mcp' },
  { category: 'Research', name: 'Exa', desc: 'Web search, page contents and code examples', url: 'https://mcp.exa.ai/mcp' },
  { category: 'Research', name: 'Tavily', desc: 'Web search, extract and crawl', url: 'https://mcp.tavily.com/mcp' },
  { category: 'Research', name: 'Ahrefs', desc: 'Keywords, backlinks, rankings and site audits', url: 'https://api.ahrefs.com/mcp/mcp' },
  { category: 'Research', name: 'Semrush', desc: 'Keywords, backlinks, traffic and competitors', url: 'https://mcp.semrush.com/v1/mcp' },
  { category: 'Research', name: 'Crustdata', desc: 'Live data on people and companies', url: 'https://install.crustdata.com/mcp' },
  { category: 'Research', name: 'Treg', desc: 'Thousands of data and AI tools behind one sign-in', url: 'https://treg.to/mcp/' },
  { category: 'Research', name: 'Hugging Face', desc: 'Models, datasets, papers and Spaces', url: 'https://huggingface.co/mcp' },

  { category: 'Work', name: 'Notion', desc: 'Search, read and edit your pages and databases', url: 'https://mcp.notion.com/mcp' },
  { category: 'Work', name: 'Linear', desc: 'Issues, projects and comments', url: 'https://mcp.linear.app/mcp' },
  { category: 'Work', name: 'Atlassian', desc: 'Jira issues and Confluence pages', url: 'https://mcp.atlassian.com/v1/mcp' },
  { category: 'Work', name: 'Asana', desc: 'Tasks, projects and goals', url: 'https://mcp.asana.com/mcp' },
  { category: 'Work', name: 'monday.com', desc: 'Boards, items and updates', url: 'https://mcp.monday.com/mcp' },
  { category: 'Work', name: 'ClickUp', desc: 'Tasks, docs and time tracking', url: 'https://mcp.clickup.com/mcp' },
  { category: 'Work', name: 'Todoist', desc: 'Tasks, projects and due dates', url: 'https://ai.todoist.net/mcp' },
  { category: 'Work', name: 'Airtable', desc: 'Bases, tables and records', url: 'https://mcp.airtable.com/mcp' },
  { category: 'Work', name: 'Granola', desc: 'Your meeting notes and transcripts', url: 'https://mcp.granola.ai/mcp' },
  { category: 'Work', name: 'Intercom', desc: 'Conversations, contacts and help articles', url: 'https://mcp.intercom.com/mcp' },
  { category: 'Work', name: 'Attio', desc: 'CRM records, lists and notes', url: 'https://mcp.attio.com/mcp' },
  { category: 'Work', name: 'Close', desc: 'CRM leads, opportunities and activities', url: 'https://mcp.close.com/mcp' },
  { category: 'Work', name: 'HubSpot', desc: 'CRM contacts, companies, deals and tickets. Needs an MCP connector you create in HubSpot', url: 'https://mcp.hubspot.com/',
    app: 'In HubSpot, go to Development → MCP Connectors → Create MCP connector, with redirect URL http://localhost:33418/callback. Paste its client ID and secret here.' },
  { category: 'Work', name: 'Calendly', desc: 'Event types, availability and bookings', url: 'https://mcp.calendly.com' },
  { category: 'Work', name: 'Cal.com', desc: 'Event types, availability and bookings', url: 'https://mcp.cal.com/mcp' },
  { category: 'Work', name: 'Zapier', desc: 'Actions in thousands of apps you connect at Zapier', url: 'https://mcp.zapier.com/api/v1/connect' },

  { category: 'Files and design', name: 'Dropbox', desc: 'Search and read your files', url: 'https://mcp.dropbox.com/mcp' },
  { category: 'Files and design', name: 'Box', desc: 'Search, read and organize your files. Needs credentials from your Box admin', url: 'https://mcp.box.com',
    app: 'Your Box admin opens Admin Console → Integrations → Box MCP server → Configure, adds integration credentials with redirect URI http://localhost:33418/callback, and gives you the client ID and secret.' },
  { category: 'Files and design', name: 'Figma', desc: 'Design files, components and variables', url: 'https://mcp.figma.com/mcp' },
  { category: 'Files and design', name: 'Canva', desc: 'Create, search and export designs', url: 'https://mcp.canva.com/mcp' },
  { category: 'Files and design', name: 'Miro', desc: 'Boards, sticky notes and diagrams', url: 'https://mcp.miro.com/' },
  { category: 'Files and design', name: 'Webflow', desc: 'Sites, pages and CMS collections', url: 'https://mcp.webflow.com/mcp' },
  { category: 'Files and design', name: 'Wix', desc: 'Your Wix sites, stores and bookings', url: 'https://mcp.wix.com/mcp' },

  { category: 'Developer', name: 'GitHub', desc: 'Repositories, issues and pull requests. Needs a personal access token: Bearer ghp_…', url: 'https://api.githubcopilot.com/mcp/', header: 'Authorization' },
  { category: 'Developer', name: 'GitLab', desc: 'Projects, issues and merge requests', url: 'https://gitlab.com/api/v4/mcp' },
  { category: 'Developer', name: 'Sentry', desc: 'Errors, issues and releases', url: 'https://mcp.sentry.dev/mcp' },
  { category: 'Developer', name: 'Vercel', desc: 'Projects, deployments and logs', url: 'https://mcp.vercel.com' },
  { category: 'Developer', name: 'Netlify', desc: 'Sites, deploys and forms', url: 'https://mcp.netlify.com/mcp' },
  { category: 'Developer', name: 'Supabase', desc: 'Projects, tables and SQL', url: 'https://mcp.supabase.com/mcp' },
  { category: 'Developer', name: 'Neon', desc: 'Postgres projects, branches and SQL', url: 'https://mcp.neon.tech/mcp' },
  { category: 'Developer', name: 'Context7', desc: 'Current docs for libraries and frameworks', url: 'https://mcp.context7.com/mcp' },
  { category: 'Developer', name: 'DeepWiki', desc: 'Ask about any public GitHub repository', url: 'https://mcp.deepwiki.com/mcp' },
  { category: 'Developer', name: 'Cloudflare docs', desc: 'Search Cloudflare’s documentation', url: 'https://docs.mcp.cloudflare.com/mcp' },
]
