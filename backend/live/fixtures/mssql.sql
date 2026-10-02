-- The objects of the MS SQL Server live tests. Each test loads this file
-- into a new database of its own, through the driver of the application.

CREATE SCHEMA hr;
GO

CREATE TABLE dbo.Orders (
    id int IDENTITY PRIMARY KEY,
    total money,
    b binary(4),
    vb varbinary(max),
    rv rowversion,
    img image
);
CREATE TABLE dbo.Lines (id int PRIMARY KEY, order_id int REFERENCES dbo.Orders (id));
CREATE TABLE hr.People (id int PRIMARY KEY, name nvarchar(50));
INSERT INTO dbo.Orders (total, b, vb, img)
VALUES (150, 0x0A1B, 0xABCDEF, 0x01), (5, 0x00, 0x, NULL);
GO

CREATE VIEW dbo.BigOrders AS SELECT id, total FROM dbo.Orders WHERE total > 100;
GO

CREATE SYNONYM dbo.OrdersSyn FOR dbo.Orders;
CREATE SYNONYM hr.PeopleSyn FOR [hr].[People];
GO

CREATE TRIGGER dbo.trg_write ON dbo.Orders AFTER INSERT, UPDATE
AS BEGIN SET NOCOUNT ON; END;
GO
CREATE TRIGGER dbo.trg_delete ON dbo.Orders FOR DELETE
AS BEGIN SET NOCOUNT ON; END;
GO
CREATE TRIGGER dbo.trg_disabled ON dbo.Orders AFTER INSERT, UPDATE, DELETE
AS BEGIN SET NOCOUNT ON; END;
GO
DISABLE TRIGGER dbo.trg_disabled ON dbo.Orders;
GO
CREATE TRIGGER dbo.trg_instead ON dbo.BigOrders INSTEAD OF INSERT, DELETE
AS BEGIN SET NOCOUNT ON; END;
GO
CREATE TRIGGER hr.trg_people ON hr.People AFTER UPDATE
AS BEGIN SET NOCOUNT ON; END;
GO

-- A trigger of the database fires on a change of the schema. It belongs to
-- no relation, so no trigger list shows it.
CREATE TRIGGER trg_schema_change ON DATABASE FOR CREATE_TABLE
AS SET NOCOUNT ON;
GO
