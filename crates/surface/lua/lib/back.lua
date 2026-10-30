-- The page a page shown over another goes back to (FILE's pages: the page FILE was
-- pressed on).

local back = {}

back.page = nil

-- Show `page` over the page on screen.
function back.over(page)
  back.page = deckard.page()
  deckard.show_page(page)
end

-- Back to the page it was shown over.
function back.go()
  deckard.show_page(back.page or "control")
  back.page = nil
end

return back
