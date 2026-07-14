library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
    generic (WIDTH : integer := 8);
    port (
        clk   : in  std_logic;
        rst_n : in  std_logic;
        q     : out unsigned(WIDTH-1 downto 0)
    );
end entity;

architecture rtl of counter is
    signal count : unsigned(WIDTH-1 downto 0) := (others => '0');
begin
    process (clk)
    begin
        if rising_edge(clk) then
            if rst_n = '0' then
                count <= (others => '0');
            else
                count <= count + 1;
            end if;
        end if;
    end process;

    q <= count;
end architecture;
