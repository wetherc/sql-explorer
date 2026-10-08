/**
 * The sample servers, rows and history that the screenshots show. No host,
 * user or row here is real: the hosts end in `.invalid`, and the customers
 * are pioneers of computing.
 */
import {
  AwsCredentialSource,
  ConstraintType,
  DbType,
  Dialect,
  MssqlAuth,
  RelationType,
  RoutineType,
  defaultConnectionOptions,
  type CellValue,
  type ColumnRef,
  type ConstraintRef,
  type DriverCapabilities,
  type EngineInfo,
  type HistoryEntry,
  type IndexRef,
  type QueryStats,
  type RoutineRef,
  type SavedConnection,
  type TableFact,
  type TableRef,
} from '@/types/api'
import type { SampleSet } from './frames'

export const ENGINES: EngineInfo[] = [
  {
    dbType: DbType.Mssql,
    label: 'MS SQL Server',
    dialect: Dialect.MsSql,
    defaultPort: 1433,
    usesHost: true,
    usesCredentials: true,
    usesDatabase: true,
    usesTls: true,
    usesFile: false,
    usesAws: false,
    supportsSchemas: true,
    supportsIntegratedSecurity: true,
    readOnly: 'intent',
  },
  {
    dbType: DbType.Athena,
    label: 'AWS Athena',
    dialect: Dialect.Athena,
    defaultPort: null,
    usesHost: false,
    usesCredentials: false,
    usesDatabase: true,
    usesTls: false,
    usesFile: false,
    usesAws: true,
    supportsSchemas: false,
    supportsIntegratedSecurity: false,
    readOnly: 'none',
  },
  {
    dbType: DbType.Postgres,
    label: 'PostgreSQL',
    dialect: Dialect.Postgres,
    defaultPort: 5432,
    usesHost: true,
    usesCredentials: true,
    usesDatabase: true,
    usesTls: true,
    usesFile: false,
    usesAws: false,
    supportsSchemas: true,
    supportsIntegratedSecurity: false,
    readOnly: 'session',
  },
  {
    dbType: DbType.Mysql,
    label: 'MySQL or MariaDB',
    dialect: Dialect.MySql,
    defaultPort: 3306,
    usesHost: true,
    usesCredentials: true,
    usesDatabase: true,
    usesTls: true,
    usesFile: false,
    usesAws: false,
    supportsSchemas: false,
    supportsIntegratedSecurity: false,
    readOnly: 'session',
  },
  {
    dbType: DbType.Sqlite,
    label: 'SQLite',
    dialect: Dialect.Sqlite,
    defaultPort: null,
    usesHost: false,
    usesCredentials: false,
    usesDatabase: false,
    usesTls: false,
    usesFile: true,
    usesAws: false,
    supportsSchemas: true,
    supportsIntegratedSecurity: false,
    readOnly: 'session',
  },
]

const NO_CAPABILITIES: DriverCapabilities = {
  supportsSchemas: false,
  supportsMultipleDatabases: true,
  supportsCancel: true,
  supportsTransactions: false,
  supportsRoutines: false,
  supportsIndexes: false,
  supportsConstraints: false,
  supportsPartitions: false,
  supportsExplain: true,
  supportsMaterializedViews: false,
  supportsForeignTables: false,
  supportsSynonyms: false,
  supportsTriggers: false,
  supportsViewTriggers: false,
  supportsEvents: false,
}

/** What each driver of the backend reports it can do. */
export const CAPABILITIES: Record<DbType, DriverCapabilities> = {
  [DbType.Athena]: { ...NO_CAPABILITIES, supportsPartitions: true },
  [DbType.Postgres]: {
    ...NO_CAPABILITIES,
    supportsSchemas: true,
    supportsMultipleDatabases: false,
    supportsTransactions: true,
    supportsRoutines: true,
    supportsIndexes: true,
    supportsConstraints: true,
    supportsMaterializedViews: true,
    supportsForeignTables: true,
  },
  [DbType.Mssql]: {
    ...NO_CAPABILITIES,
    supportsSchemas: true,
    supportsTransactions: true,
    supportsRoutines: true,
    supportsIndexes: true,
    supportsConstraints: true,
    supportsSynonyms: true,
  },
  [DbType.Mysql]: {
    ...NO_CAPABILITIES,
    supportsTransactions: true,
    supportsRoutines: true,
    supportsIndexes: true,
    supportsConstraints: true,
    supportsEvents: true,
  },
  [DbType.Sqlite]: {
    ...NO_CAPABILITIES,
    supportsSchemas: true,
    supportsMultipleDatabases: false,
    supportsTransactions: true,
    supportsIndexes: true,
    supportsConstraints: true,
  },
}

function connection(
  fields: Pick<SavedConnection, 'id' | 'name' | 'dbType' | 'group'> & Partial<SavedConnection>,
  options: Partial<SavedConnection['options']> = {},
): SavedConnection {
  return {
    host: null,
    port: null,
    user: null,
    database: null,
    color: null,
    ...fields,
    options: { ...defaultConnectionOptions(), ...options },
  }
}

export const CONNECTIONS: SavedConnection[] = [
  connection(
    { id: 'lake', name: 'Lake (AWS Athena)', dbType: DbType.Athena, group: 'Analytics' },
    {
      awsRegion: 'eu-west-1',
      awsCredentialSource: AwsCredentialSource.Chain,
      athenaWorkgroup: 'primary',
      athenaCatalog: 'AwsDataCatalog',
    },
  ),
  connection({
    id: 'reporting',
    name: 'Reporting (MySQL)',
    dbType: DbType.Mysql,
    group: 'Analytics',
    host: 'mysql.demo.invalid',
    port: 3306,
    user: 'report',
    database: 'reporting',
  }),
  connection(
    { id: 'archive', name: 'Archive (SQLite)', dbType: DbType.Sqlite, group: 'Local' },
    { filePath: '/home/demo/data/archive.sqlite' },
  ),
  connection({
    id: 'shop',
    name: 'Shop (PostgreSQL)',
    dbType: DbType.Postgres,
    group: 'Production',
    host: 'db.demo.invalid',
    port: 5432,
    user: 'shop_reader',
    database: 'shop',
  }),
  connection(
    {
      id: 'warehouse',
      name: 'Warehouse (MS SQL Server)',
      dbType: DbType.Mssql,
      group: 'Production',
      host: 'sql.demo.invalid',
      port: 1433,
      database: 'Warehouse',
    },
    { mssqlAuth: MssqlAuth.EntraAzureCli },
  ),
]

/** The connections that are open when the window opens. */
export const ACTIVE = ['shop', 'lake']

/** One relation of the sample catalog, with what the tree and the dialogs show. */
export interface SampleRelation extends TableRef {
  columns: ColumnRef[]
  indexes?: IndexRef[]
  constraints?: ConstraintRef[]
  facts?: TableFact[]
}

/** The schemas of one database, or `''` for an engine without schemas. */
export interface SampleDatabase {
  name: string
  schemas: Record<string, { relations: SampleRelation[]; routines?: RoutineRef[] }>
}

function column(name: string, dataType: string, nullable = false, isPrimaryKey = false): ColumnRef {
  return { name, dataType, nullable, isPrimaryKey, isGenerated: false }
}

function table(name: string, columns: ColumnRef[], extra: Partial<SampleRelation> = {}) {
  return { name, relationType: RelationType.Table, columns, ...extra }
}

const ORDERS: SampleRelation = table(
  'orders',
  [
    column('order_id', 'bigint', false, true),
    column('customer_id', 'bigint'),
    column('placed_at', 'timestamptz'),
    column('status', 'text'),
    column('total_amount', 'numeric(12,2)'),
    column('currency', 'char(3)'),
    column('discount_code', 'text', true),
    column('shipped_at', 'timestamptz', true),
  ],
  {
    indexes: [
      { name: 'orders_pkey', columns: ['order_id'], unique: true, primary: true, included: [] },
      {
        name: 'orders_customer_idx',
        columns: ['customer_id'],
        unique: false,
        primary: false,
        included: [],
      },
      {
        name: 'orders_placed_status_idx',
        columns: ['placed_at', 'status'],
        unique: false,
        primary: false,
        included: [],
      },
    ],
    constraints: [
      {
        name: 'orders_pkey',
        constraintType: ConstraintType.PrimaryKey,
        columns: ['order_id'],
        detail: null,
      },
      {
        name: 'orders_customer_id_fkey',
        constraintType: ConstraintType.ForeignKey,
        columns: ['customer_id'],
        detail: 'REFERENCES customers (customer_id)',
      },
      {
        name: 'orders_total_check',
        constraintType: ConstraintType.Check,
        columns: ['total_amount'],
        detail: 'CHECK (total_amount >= 0)',
      },
    ],
    // The facts in the words of the PostgreSQL driver.
    facts: [
      { name: 'Rows', value: 'about 1284902' },
      { name: 'Size', value: '412.0 MB' },
      { name: 'Owner', value: 'shop_owner' },
    ],
  },
)

export const CATALOG: Record<string, SampleDatabase[]> = {
  shop: [
    {
      name: 'shop',
      schemas: {
        public: {
          relations: [
            table('customers', [
              column('customer_id', 'bigint', false, true),
              column('name', 'text'),
              column('email', 'text'),
              column('country', 'char(2)'),
              column('created_at', 'timestamptz'),
            ]),
            table('order_items', [
              column('order_id', 'bigint', false, true),
              column('line_no', 'integer', false, true),
              column('product_id', 'bigint'),
              column('quantity', 'integer'),
              column('unit_price', 'numeric(12,2)'),
            ]),
            ORDERS,
            table('products', [
              column('product_id', 'bigint', false, true),
              column('sku', 'text'),
              column('title', 'text'),
              column('price', 'numeric(12,2)'),
            ]),
            table('shipments', [
              column('shipment_id', 'bigint', false, true),
              column('order_id', 'bigint'),
              column('carrier', 'text'),
              column('shipped_at', 'timestamptz'),
            ]),
            {
              name: 'monthly_revenue',
              relationType: RelationType.View,
              columns: [column('month', 'date'), column('revenue', 'numeric')],
            },
            {
              name: 'open_orders',
              relationType: RelationType.View,
              columns: [column('order_id', 'bigint'), column('status', 'text')],
            },
          ],
          routines: [
            { name: 'refresh_revenue', routineType: RoutineType.Procedure },
            { name: 'order_total', routineType: RoutineType.Function },
          ],
        },
        sales: { relations: [] },
        staging: { relations: [] },
      },
    },
    { name: 'shop_staging', schemas: { public: { relations: [] } } },
  ],
  lake: [
    {
      name: 'lake',
      schemas: {
        '': {
          relations: [
            table('clickstream', [
              column('event_day', 'date'),
              column('channel', 'varchar'),
              column('session_id', 'varchar'),
              column('order_id', 'bigint', true),
              column('revenue', 'decimal(14,2)', true),
            ]),
            table('page_views', [column('event_day', 'date'), column('url', 'varchar')]),
          ],
        },
      },
    },
  ],
}

const CUSTOMERS = [
  ['Ada Lovelace', 'ada', 'GB'],
  ['Grace Hopper', 'grace', 'NL'],
  ['Alan Turing', 'alan', 'GB'],
  ['Edsger Dijkstra', 'edsger', 'NL'],
  ['Barbara Liskov', 'barbara', 'DE'],
  ['Tony Hoare', 'tony', 'FR'],
  ['Frances Allen', 'frances', 'SE'],
  ['Ken Thompson', 'ken', 'IT'],
  ['Karen Spärck Jones', 'karen', 'ES'],
  ['Donald Knuth', 'donald', 'PL'],
] as const
const STATUSES = ['delivered', 'shipped', 'packing', 'placed', 'delivered', 'shipped', 'delivered']
const DISCOUNTS = [null, 'SUMMER10', null, 'WELCOME', null, null, 'SUMMER10']

function pad(value: number): string {
  return String(value).padStart(2, '0')
}

/** The 50 newest orders, one every 1 h 23 min going back from 30 July. */
function orderRows(): CellValue[][] {
  const start = Date.UTC(2026, 6, 30, 21, 0, 12)
  return Array.from({ length: 50 }, (_, index) => {
    const placed = new Date(start - index * 83 * 60_000)
    const [name, user, country] = CUSTOMERS[index % CUSTOMERS.length]!
    const day = `${placed.getUTCFullYear()}-${pad(placed.getUTCMonth() + 1)}-${pad(placed.getUTCDate())}`
    const time = `${pad(placed.getUTCHours())}:${pad(placed.getUTCMinutes())}:12`
    return [
      918240 - index,
      `${day} ${time}+02`,
      `${name} <${user}@demo.invalid>`,
      country,
      STATUSES[index % STATUSES.length]!,
      (index % 7) + 1,
      Number((48.5 + ((index * 137.33) % 900)).toFixed(2)),
      DISCOUNTS[index % DISCOUNTS.length]!,
    ]
  })
}

/** What one sample statement gives when it runs. */
export interface SampleRun {
  /** True when the text of a statement contains a fragment that only the
   *  statement of this sample run has, such as `lake.clickstream`. */
  matches: (query: string) => boolean
  sets: SampleSet[]
  elapsedMs: number
  stats?: QueryStats
}

const lower = (query: string) => query.toLowerCase()

export const RUNS: SampleRun[] = [
  {
    matches: (query) => lower(query).includes('left join public.order_items'),
    elapsedMs: 132,
    sets: [
      {
        columns: [
          { name: 'order_id', typeName: 'bigint' },
          { name: 'placed_at', typeName: 'timestamptz' },
          { name: 'customer', typeName: 'text' },
          { name: 'country', typeName: 'char(2)' },
          { name: 'status', typeName: 'text' },
          { name: 'items', typeName: 'integer' },
          { name: 'total_amount', typeName: 'numeric(12,2)' },
          { name: 'discount_code', typeName: 'text' },
        ],
        rows: orderRows(),
      },
    ],
  },
  {
    matches: (query) => lower(query).includes('lake.clickstream'),
    elapsedMs: 2420,
    stats: { scannedBytes: 1_530_000_000, engineMs: 2210, queueMs: 140, resultReused: false },
    sets: [
      {
        columns: [
          { name: 'event_day', typeName: 'date' },
          { name: 'channel', typeName: 'varchar' },
          { name: 'sessions', typeName: 'bigint' },
          { name: 'orders', typeName: 'bigint' },
          { name: 'revenue', typeName: 'decimal(14,2)' },
        ],
        rows: [
          ['2026-07-28', 'search', 184920, 4120, 498231.4],
          ['2026-07-28', 'email', 60184, 2044, 244912.15],
          ['2026-07-28', 'social', 41288, 912, 98431.7],
          ['2026-07-29', 'search', 190441, 4318, 521004.85],
          ['2026-07-29', 'email', 58712, 1988, 238114.6],
          ['2026-07-29', 'social', 43907, 1004, 106722.3],
          ['2026-07-30', 'search', 201338, 4602, 553918.2],
          ['2026-07-30', 'email', 61044, 2110, 251880.45],
          ['2026-07-30', 'social', 45120, 1088, 114093.9],
        ],
      },
    ],
  },
]

/** The lines of the estimated plan of the orders statement. */
export const PLAN_LINES = [
  'Limit  (cost=18422.51..18422.63 rows=50 width=96)',
  '  ->  Sort  (cost=18422.51..18455.09 rows=13031 width=96)',
  '        Sort Key: o.placed_at DESC',
  '        ->  HashAggregate  (cost=17989.44..18119.75 rows=13031 width=96)',
  '              Group Key: o.order_id, c.name, c.email, c.country',
  '              ->  Hash Join  (cost=412.88..17859.13 rows=13031 width=88)',
  '                    Hash Cond: (o.customer_id = c.customer_id)',
  '                    ->  Index Scan using orders_placed_status_idx on orders o  (cost=0.43..17204.02 rows=13031 width=64)',
  "                          Index Cond: (placed_at >= (now() - '30 days'::interval))",
  '                    ->  Hash  (cost=298.06..298.06 rows=9186 width=40)',
  '                          ->  Seq Scan on customers c  (cost=0.00..298.06 rows=9186 width=40)',
]

const ORDERS_QUERY = `-- The orders of the last 30 days, with the customer of each one.
select o.order_id,
       o.placed_at,
       c.name || ' <' || c.email || '>' as customer,
       c.country,
       o.status,
       count(i.line_no) as items,
       o.total_amount,
       o.discount_code
from public.orders o
join public.customers c on c.customer_id = o.customer_id
left join public.order_items i on i.order_id = o.order_id
where o.placed_at >= now() - interval '30 days'
group by o.order_id, c.name, c.email, c.country
order by o.placed_at desc
limit 50;
`

const COUNTRY_QUERY = `-- A statement that contains names asks for their values before it runs.
select o.order_id,
       o.placed_at,
       o.status,
       o.total_amount
from public.orders o
join public.customers c on c.customer_id = o.customer_id
where c.country = :country
  and o.total_amount >= :least
  and o.placed_at >= :since
order by o.total_amount desc;
`

const CHANNEL_QUERY = `-- Athena reports the data it scanned, and the status bar prices it.
select event_day,
       channel,
       count(*) as sessions,
       count_if(order_id is not null) as orders,
       sum(revenue) as revenue
from lake.clickstream
where event_day between date '2026-07-28' and date '2026-07-30'
group by event_day, channel
order by event_day, channel;
`

/** The tabs of the last session. */
export const WORKSPACE = {
  tabs: [
    {
      id: 'tab-orders',
      title: 'Orders of the month',
      query: ORDERS_QUERY,
      connectionId: 'shop',
      dirty: false,
      params: [],
      filePath: null,
    },
    {
      id: 'tab-country',
      title: 'Orders of one country',
      query: COUNTRY_QUERY,
      connectionId: 'shop',
      dirty: false,
      params: [
        { name: 'country', valueType: 'text', text: 'NL' },
        { name: 'least', valueType: 'number', text: '250' },
        { name: 'since', valueType: 'text', text: '2026-07-01' },
      ],
      filePath: null,
    },
    {
      id: 'tab-channel',
      title: 'Sessions of each channel',
      query: CHANNEL_QUERY,
      connectionId: 'lake',
      dirty: false,
      params: [],
      filePath: null,
    },
  ],
  activeTabId: 'tab-orders',
}

function entry(
  index: number,
  connectionId: string,
  query: string,
  minutesAgo: number,
  rowCount: number,
  error: string | null = null,
): HistoryEntry {
  const connectionName = CONNECTIONS.find((item) => item.id === connectionId)!.name
  return {
    id: `history-${index}`,
    connectionId,
    connectionName,
    query,
    ranAt: new Date(Date.UTC(2026, 7, 11, 9, 30) - minutesAgo * 60_000).toISOString(),
    elapsedMs: 40 + index * 37,
    rowCount,
    succeeded: error === null,
    error,
  }
}

export const HISTORY: HistoryEntry[] = [
  entry(1, 'shop', ORDERS_QUERY, 2, 50),
  entry(2, 'shop', ORDERS_QUERY, 9, 50),
  entry(
    3,
    'shop',
    'select country, count(*) as orders\nfrom public.customers\ngroup by 1;',
    17,
    12,
  ),
  entry(4, 'lake', CHANNEL_QUERY, 25, 9),
  entry(
    5,
    'shop',
    "update public.orders set status = 'shipped' where order_id = 918233;",
    41,
    0,
    'permission denied for table orders',
  ),
  entry(6, 'shop', 'select * from public.shipments where shipped_at is null;', 58, 214),
]
