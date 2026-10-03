-- The objects of the MySQL and MariaDB live tests. Each test loads this file
-- into a new database of its own, through the driver of the application.

CREATE TABLE orders (id int PRIMARY KEY, total decimal(10, 2), note text);
CREATE VIEW big_orders AS SELECT id, total FROM orders WHERE total > 100;

CREATE TRIGGER bi_orders BEFORE INSERT ON orders
    FOR EACH ROW SET NEW.note = COALESCE(NEW.note, 'x');
-- FOLLOWS and PRECEDES give a firing order that differs from the order of
-- the names: bi_stamp, bi_orders, bi_check.
CREATE TRIGGER bi_check BEFORE INSERT ON orders
    FOR EACH ROW FOLLOWS bi_orders SET NEW.total = COALESCE(NEW.total, 0);
CREATE TRIGGER bi_stamp BEFORE INSERT ON orders
    FOR EACH ROW PRECEDES bi_orders SET @inserted = NEW.id;
CREATE TRIGGER au_orders AFTER UPDATE ON orders
    FOR EACH ROW SET @changed = NEW.id;
CREATE TRIGGER bd_orders BEFORE DELETE ON orders
    FOR EACH ROW SET @gone = OLD.id;

CREATE EVENT ev_daily ON SCHEDULE EVERY 1 DAY
    DO DELETE FROM orders WHERE id < 0;
CREATE EVENT ev_once ON SCHEDULE AT '2030-06-01 12:34:56'
    ON COMPLETION PRESERVE DO SELECT 1;
CREATE EVENT ev_off ON SCHEDULE EVERY 5 MINUTE DISABLE DO SELECT 1;

-- A body of more than one statement needs another terminator.
DELIMITER $$
CREATE TRIGGER bu_orders BEFORE UPDATE ON orders FOR EACH ROW
BEGIN
    SET NEW.note = CONCAT(COALESCE(OLD.note, ''), ';');
    SET @updated = NEW.id;
END$$
CREATE EVENT ev_body ON SCHEDULE EVERY 1 HOUR DO
BEGIN
    DELETE FROM orders WHERE id < 0;
    SET @swept = 1;
END$$
DELIMITER ;

-- A body that contains $$ needs a terminator other than $$.
DELIMITER //
CREATE EVENT ev_mark ON SCHEDULE EVERY 1 WEEK DO
BEGIN
    SET @mark = '$$';
    SET @mark$$ = 1;
END//
DELIMITER ;
