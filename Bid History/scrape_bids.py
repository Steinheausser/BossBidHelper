import pandas as pd
from playwright.sync_api import sync_playwright
import time
import os

def scrape_bids():
    with sync_playwright() as p:
        # Launch Chromium in non-headless mode so you can see and interact with the page
        browser = p.chromium.launch(headless=False)
        context = browser.new_context()
        page = context.new_page()

        print("Navigating to BOSS Bidding System...")
        page.goto("https://boss.intranet.smu.edu.sg/OverallResults.aspx")

        # Pause and wait for the user to manually log in and perform the search
        print("\n" + "="*60)
        print("ACTION REQUIRED:")
        print("1. Log in to the BOSS system if prompted.")
        print("2. Search for the '2025-26 Term 2' data (or whichever term you want).")
        print("3. Wait until the results table fully loads.")
        print("4. Press ENTER in this console to begin the automated scraping.")
        print("="*60 + "\n")
        input("Press ENTER when you are ready to start scraping...")

        all_data = []
        page_num = 1
        
        while True:
            print(f"Scraping page {page_num}...")
            
            # Ensure the loading spinner is hidden (it usually has the ID 'UpdateProgress1')
            page.wait_for_selector("#UpdateProgress1", state="hidden")
            time.sleep(1) # Small buffer to ensure the DOM is fully re-rendered

            # Extract the rows from the grid
            rows = page.locator("table.rgMasterTable tbody tr").all()
            for row in rows:
                cols = row.locator("td").all_inner_texts()
                # Clean up leading/trailing whitespaces
                cols = [c.strip() for c in cols]
                # Filter out debris rows (like pagination) which don't have exactly 16 columns
                if len(cols) == 16 and "Page size" not in cols[0]:
                    all_data.append(cols)

            # Locate the "Next Page" button
            next_btn = page.locator("input.rgPageNext")
            
            # If the button doesn't exist, we're likely done
            if next_btn.count() == 0:
                print("No 'Next' button found. Finishing.")
                break
                
            # In Telerik Grids, disabled pagination buttons often have 'onclick="return false;"'
            onclick_val = next_btn.get_attribute("onclick")
            if onclick_val == "return false;":
                print("Reached the last page. Finishing.")
                break
                
            # Click next page and wait for the subsequent network POST request to complete
            with page.expect_response(lambda response: "OverallResults.aspx" in response.url and response.request.method == "POST", timeout=30000):
                next_btn.click()
            
            page_num += 1

        print(f"\nTotal rows scraped: {len(all_data)}")
        
        if len(all_data) > 0:
            # The 16 columns exactly as they appear in the HTML table
            raw_columns = [
                "Term", "Session", "Bidding Window", "Course", "Description", 
                "Sect", "Median", "Min", "Vacancy", "Open", "Bef Proc", 
                "Aft Proc", "DICE", "Enrolled", "Instructor", "School"
            ]
            
            df = pd.DataFrame(all_data, columns=raw_columns)
            
            # Rename columns to match the older xlsx files
            rename_map = {
                "Course": "Course Code",
                "Sect": "Section",
                "Open": "Opening Vacancy",
                "Bef Proc": "Before Process Vacancy",
                "DICE": "D.I.C.E",
                "Aft Proc": "After Process Vacancy",
                "Enrolled": "Enrolled Students",
                "Median": "Median Bid",
                "Min": "Min Bid",
                "School": "School/Department"
            }
            df.rename(columns=rename_map, inplace=True)
            
            # Reorder columns to match the older xlsx files
            target_columns = [
                'Term', 'Session', 'Bidding Window', 'Course Code', 'Description', 
                'Section', 'Vacancy', 'Opening Vacancy', 'Before Process Vacancy', 
                'D.I.C.E', 'After Process Vacancy', 'Enrolled Students', 
                'Median Bid', 'Min Bid', 'Instructor', 'School/Department'
            ]
            df = df[target_columns]
            
            # Save the file in the same directory as this script
            script_dir = os.path.dirname(os.path.abspath(__file__))
            output_path = os.path.join(script_dir, "2025-26_T2.xlsx")
            
            df.to_excel(output_path, index=False)
            print(f"Successfully saved data to: {output_path}")
        else:
            print("No data was scraped. Please make sure the table was visible before you pressed ENTER.")

        browser.close()

if __name__ == "__main__":
    scrape_bids()
