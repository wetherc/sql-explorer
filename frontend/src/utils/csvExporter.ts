// src/utils/csvExporter.ts

/**
 * Converts an array of objects into a CSV string.
 * Handles headers and proper escaping for commas and double quotes within data.
 * @param data - An array of objects to convert. Each object represents a row.
 * @returns A CSV formatted string.
 */
export function exportToCsv(data: Record<string, unknown>[]): string {
  if (!data || data.length === 0) {
    return '';
  }

  const headers = Object.keys(data[0]!);
  const csvRows = [];

  // Add headers
  csvRows.push(headers.map(header => escapeCsvValue(header)).join(','));

  // Add data rows
  for (const row of data) {
    const values = headers.map(header => escapeCsvValue(row[header]));
    csvRows.push(values.join(','));
  }

      return csvRows.join('\n');
}

/**
 * Escapes a single value for CSV output.
 * If the value contains a comma, double quote, or newline, it is enclosed in double quotes.
 * Any double quotes within the value are escaped by doubling them.
 * @param value - The value to escape.
 * @returns The escaped string.
 */
function escapeCsvValue(value: unknown): string {
  if (value === null || value === undefined) {
    return '';
  }
  let stringValue = String(value);

  // Check if value contains comma, double quote, or newline
    if (stringValue.includes(' ') || stringValue.includes(',') || stringValue.includes('"') || stringValue.includes('\n') || stringValue.includes('\r')) {
    // Escape double quotes by doubling them
    stringValue = stringValue.replace(/"/g, '""');
    // Enclose the value in double quotes
    return `"${stringValue}"`;
  }
  return stringValue;
}

/**
 * Triggers a download of a CSV string as a file.
 * @param csvString - The CSV content.
 * @param filename - The name of the file to download.
 */
export function downloadCsv(csvString: string, filename: string): void {
  const blob = new Blob([csvString], { type: 'text/csv;charset=utf-8;' });
  const link = document.createElement('a');
  if (link.download !== undefined) { // Feature detection
    const url = URL.createObjectURL(blob);
    link.setAttribute('href', url);
    link.setAttribute('download', filename);
    link.style.visibility = 'hidden';
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url); // Clean up the URL object
  } else {
    // Fallback for older browsers
    console.error('Download not supported in this browser.');
  }
}
