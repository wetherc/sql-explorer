export interface ResultSet {
  columns: string[];
  rows: Record<string, any>[];
}

export interface QueryResponse {
  results: ResultSet[];
  messages: string[];
}
