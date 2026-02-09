export interface ResultSet {
  columns: string[];
  rows: Record<string, unknown>[];
}

export interface QueryResponse {
  results: ResultSet[];
  messages: string[];
}
