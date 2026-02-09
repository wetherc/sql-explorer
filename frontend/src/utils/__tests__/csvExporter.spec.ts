// frontend/src/utils/__tests__/csvExporter.spec.ts
import { describe, it, expect, vi, beforeAll, beforeEach } from 'vitest';
import { exportToCsv, downloadCsv } from '../csvExporter';

describe('csvExporter', () => {
  describe('exportToCsv', () => {
    it('should convert a simple array of objects to CSV', () => {
      const data = [
        { id: 1, name: 'Alice', age: 30 },
        { id: 2, name: 'Bob', age: 24 },
      ];
      const expectedCsv = `id,name,age
1,Alice,30
2,Bob,24`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should handle values with commas correctly', () => {
      const data = [
        { id: 1, name: 'Alice, Smith', city: 'New York' },
      ];
      const expectedCsv = `id,name,city
1,"Alice, Smith","New York"`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should handle values with double quotes correctly', () => {
      const data = [
        { id: 1, description: 'He said "Hello!"' },
      ];
      const expectedCsv = `id,description
1,"He said ""Hello!"""`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should handle values with commas and double quotes', () => {
      const data = [
        { id: 1, description: 'Text with, "quotes" and commas' },
      ];
      const expectedCsv = `id,description
1,"Text with, ""quotes"" and commas"`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should handle empty string values', () => {
      const data = [
        { id: 1, name: 'Alice', comment: '' },
      ];
      const expectedCsv = `id,name,comment
1,Alice,`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should handle null and undefined values', () => {
      const data = [
        { id: 1, name: 'Alice', comment: null },
        { id: 2, name: 'Bob', comment: undefined },
      ];
      const expectedCsv = `id,name,comment
1,Alice,
2,Bob,`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should handle numbers and booleans', () => {
      const data = [
        { id: 1, active: true, price: 99.99 },
      ];
      const expectedCsv = `id,active,price
1,true,99.99`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should return empty string for empty data array', () => {
      const data: Record<string, unknown>[] = [];
      expect(exportToCsv(data)).toBe('');
    });

    it('should handle headers with special characters', () => {
      const data = [
        { 'Product Name': 'Laptop', 'Item ID': 101 },
      ];
      const expectedCsv = `"Product Name","Item ID"
Laptop,101`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });

    it('should handle values with newlines', () => {
      const data = [
        { id: 1, description: `Line 1
Line 2` },
      ];
      const expectedCsv = `id,description
1,"Line 1
Line 2"`;
      expect(exportToCsv(data)).toBe(expectedCsv);
    });
  });

  describe('downloadCsv', () => {
    // Mock the DOM elements and functions used for downloading
    const mockCreateObjectURL = vi.fn();
    const mockRevokeObjectURL = vi.fn();
    const mockAppendChild = vi.fn();
    const mockRemoveChild = vi.fn();
    const mockClick = vi.fn();

    interface MockedAnchorElement {
      href: string;
      download: string;
      style: Record<string, string>;
      setAttribute: (attr: string, value: string) => void;
      click: () => void;
    }

    beforeAll(() => {
      // @ts-expect-error global.URL is mocked
      global.URL = {
        createObjectURL: mockCreateObjectURL,
        revokeObjectURL: mockRevokeObjectURL,
      };

      Object.defineProperty(document, 'createElement', {
        value: (tagName: string) => {
          if (tagName === 'a') {
            const link: MockedAnchorElement = {
              href: '',
              download: '',
              style: {},
              setAttribute: (attr: string, value: string) => {
                if (attr === 'href') link.href = value;
                if (attr === 'download') link.download = value;
              },
              click: mockClick,
            };
            return link;
          }
          return null;
        },
        writable: true,
      });

      Object.defineProperty(document.body, 'appendChild', {
        value: mockAppendChild,
        writable: true,
      });
      Object.defineProperty(document.body, 'removeChild', {
        value: mockRemoveChild,
        writable: true,
      });
    });

    beforeEach(() => {
      vi.clearAllMocks();
      mockCreateObjectURL.mockReturnValue('blob:mock-url');
    });

    it('should trigger a file download with the correct CSV content and filename', () => {
      const csvContent = `a,b
1,2`;
      const filename = 'test.csv';

      downloadCsv(csvContent, filename);

      expect(mockCreateObjectURL).toHaveBeenCalledTimes(1);
      expect(mockCreateObjectURL).toHaveBeenCalledWith(expect.any(Blob));

      expect(mockAppendChild).toHaveBeenCalledTimes(1);
      expect(mockClick).toHaveBeenCalledTimes(1);
      expect(mockRemoveChild).toHaveBeenCalledTimes(1);

      const linkElement = mockAppendChild.mock.calls[0][0];
      expect(linkElement.download).toBe(filename);
      expect(linkElement.href).toBe('blob:mock-url');

      expect(mockRevokeObjectURL).toHaveBeenCalledTimes(1);
      expect(mockRevokeObjectURL).toHaveBeenCalledWith('blob:mock-url');
    });

    it('should log an error if download is not supported', () => {
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      // Simulate a browser where download is not supported
      Object.defineProperty(document, 'createElement', {
        value: (tagName: string) => {
          if (tagName === 'a') {
            return {
              download: undefined, // Simulate no download attribute support
              setAttribute: () => {},
              click: () => {},
            };
          }
          return null;
        },
        writable: true,
      });

      downloadCsv('a,b', 'test.csv');
      expect(consoleErrorSpy).toHaveBeenCalledWith('Download not supported in this browser.');

      consoleErrorSpy.mockRestore(); // Clean up the spy
    });
  });
});
